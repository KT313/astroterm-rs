//! Complete-request reuse, lifecycle invalidation and independent observer provenance.
use crate::{astro::{J2000, Observer}, cache::CacheConfig, model::{FrameTime, ModelFamily, StateRequest},
    sky::{begin_solar_system_frame, prepare_observer_inputs, update_solar_system},
    state::{SimulationState, ObserverPreparationCache}, timing::StepTimes};

fn frame(simulation: &mut SimulationState, observer: &mut ObserverPreparationCache, time: FrameTime, site: Observer) {
    let mut times = StepTimes::default();
    begin_solar_system_frame(simulation, observer, time, site, &mut times).unwrap();
    prepare_observer_inputs(observer, simulation, time, site, &mut times).unwrap();
    assert!(!simulation.group.has_been_invalidated);
    assert_eq!(simulation.group.calculated_at, Some(time.tt));
}

fn sample_allocations(simulation: &SimulationState) -> [(usize, usize, usize); 6] {
    [
        (simulation.planets.as_ptr() as usize, simulation.planets.len(), simulation.planets.capacity()),
        (simulation.moon.as_ptr() as usize, simulation.moon.len(), simulation.moon.capacity()),
        (simulation.orientation.as_ptr() as usize, simulation.orientation.len(), simulation.orientation.capacity()),
        (simulation.planet_work.as_ptr() as usize, simulation.planet_work.len(), simulation.planet_work.capacity()),
        (simulation.moon_work.as_ptr() as usize, simulation.moon_work.len(), simulation.moon_work.capacity()),
        (simulation.orientation_work.as_ptr() as usize, simulation.orientation_work.len(), simulation.orientation_work.capacity()),
    ]
}

#[test]
fn paused_complete_group_reuses_samples_and_work_without_publication_or_copy() {
    for config in [CacheConfig::default(), CacheConfig::parse("[groups.planetary_samples]\nmax_age_seconds=0\n[groups.lunar_samples]\nmax_age_seconds=0\n[groups.slow_orientation]\nmax_age_seconds=0").unwrap()] {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(&config);
        let mut observer = ObserverPreparationCache::new(config);
        let time = FrameTime::from_utc(J2000);
        frame(&mut simulation, &mut observer, time, Observer::default());
        let saved = simulation.clone();
        let allocations = sample_allocations(&simulation);
        let observer_counts = observer.stats();
        for _ in 0..3 {
            frame(&mut simulation, &mut observer, time, Observer::default());
            assert_eq!(simulation, saved);
            assert_eq!(sample_allocations(&simulation), allocations);
            assert_eq!(observer.stats(), observer_counts);
        }
    }
}

#[test]
fn time_site_ut1_model_and_invalidation_restart_the_complete_group() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let original = FrameTime::from_utc(J2000);
    let mut site = Observer::default();
    frame(&mut simulation, &mut observer, original, site);
    for time in [FrameTime { tt: original.tt.next_up(), ..original }, original, FrameTime { ut1: original.ut1 + 0.001, ..original }] {
        let counts = simulation.refresh_counts;
        frame(&mut simulation, &mut observer, time, site);
        assert!(simulation.refresh_counts.planets > counts.planets);
        assert!(simulation.refresh_counts.moon > counts.moon);
        assert!(simulation.refresh_counts.orientation > counts.orientation);
    }
    site.longitude += 0.3;
    let generation = simulation.group.request_generation;
    frame(&mut simulation, &mut observer, original, site);
    assert!(simulation.group.request_generation > generation);
    for model in [ModelFamily::Planets, ModelFamily::Moon, ModelFamily::Orientation] {
        let generation = simulation.group.request_generation;
        simulation.set_model_version(model, 7);
        frame(&mut simulation, &mut observer, original, site);
        assert!(simulation.group.request_generation > generation);
    }
    let generation = simulation.group.request_generation;
    simulation.invalidate();
    frame(&mut simulation, &mut observer, original, site);
    assert!(simulation.group.request_generation > generation);
}

#[test]
fn disabled_families_and_exact_mode_recalculate_paused_frames() {
    for config in [CacheConfig::disabled(), CacheConfig::parse("[groups.lunar_samples]\nenabled=false").unwrap(),
        CacheConfig::parse("[groups.observer_state]\nenabled=false").unwrap(),
        CacheConfig::parse("[groups.solar_system_observation]\nenabled=false").unwrap()] {
        let mut simulation = SimulationState::default(); simulation.configure_cache(&config);
        let mut observer = ObserverPreparationCache::new(config);
        let time = FrameTime::from_utc(J2000);
        frame(&mut simulation, &mut observer, time, Observer::default());
        let before = simulation.refresh_counts;
        frame(&mut simulation, &mut observer, time, Observer::default());
        assert!(simulation.refresh_counts.planets > before.planets);
        assert!(simulation.refresh_counts.moon > before.moon);
    }
    let mut simulation = SimulationState::exact();
    let mut observer = ObserverPreparationCache::default();
    frame(&mut simulation, &mut observer, FrameTime::from_utc(J2000), Observer::default());
    let before = simulation.refresh_counts;
    frame(&mut simulation, &mut observer, FrameTime::from_utc(J2000), Observer::default());
    assert!(simulation.refresh_counts.planets > before.planets);
}

#[test]
fn reception_is_incomplete_and_missing_emission_or_observer_storage_cannot_reuse_completion() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let time = FrameTime::from_utc(J2000);
    let site = Observer::default();
    let mut times = StepTimes::default();
    begin_solar_system_frame(&mut simulation, &mut observer, time, site, &mut times).unwrap();
    assert!(simulation.group.complete_key.is_none());
    assert!(simulation.group.has_been_invalidated);
    prepare_observer_inputs(&mut observer, &mut simulation, time, site, &mut times).unwrap();
    assert!(simulation.group.complete_key.is_some());
    update_solar_system(&mut simulation, time, &[StateRequest { body: crate::astro::models::BodyId::Neptune, tt: time.tt - 1.0 }], &mut times).unwrap();
    assert!(simulation.group.complete_key.is_none());
    frame(&mut simulation, &mut observer, time, site);
    let generation = simulation.group.request_generation;
    observer.bodies.invalidate();
    frame(&mut simulation, &mut observer, time, site);
    assert!(simulation.group.request_generation > generation);
    let generation = simulation.group.request_generation;
    let mut other = ObserverPreparationCache::default();
    frame(&mut simulation, &mut other, time, site);
    assert!(simulation.group.request_generation > generation);
    assert!(observer.completed_observer(simulation.request_token()).is_none());
}

#[test]
fn failed_request_cannot_publish_and_retry_matches_fresh_preparation() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let time = FrameTime::from_utc(J2000);
    let site = Observer::default();
    frame(&mut simulation, &mut observer, time, site);
    let mut times = StepTimes::default();
    let bad = FrameTime { tt: f64::NAN, ..time };
    assert!(begin_solar_system_frame(&mut simulation, &mut observer, bad, site, &mut times).is_err());
    assert!(simulation.group.has_been_invalidated);
    assert!(simulation.group.complete_key.is_none());
    frame(&mut simulation, &mut observer, time, site);
    let mut reference = SimulationState::default();
    let mut reference_observer = ObserverPreparationCache::default();
    frame(&mut reference, &mut reference_observer, time, site);
    assert_eq!(observer.bodies.value().planets, reference_observer.bodies.value().planets);
    assert_eq!(observer.bodies.value().moon, reference_observer.bodies.value().moon);
    assert_eq!(observer.light_time.value(), reference_observer.light_time.value());
}

#[test]
fn reordering_retained_samples_invalidates_completion_without_new_evaluations() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let time = FrameTime::from_utc(J2000);
    frame(&mut simulation, &mut observer, time, Observer::default());
    let body = crate::astro::models::BodyId::Neptune;
    let emission = observer.light_time.value().emission_tt[body as usize];
    let before = simulation.refresh_counts;
    update_solar_system(&mut simulation, time, &[StateRequest { body, tt: emission }], &mut StepTimes::default()).unwrap();
    assert_eq!(simulation.refresh_counts, before);
    assert!(simulation.group.has_been_invalidated);
    assert!(simulation.group.complete_key.is_none());
}

#[test]
fn failed_emission_preparation_leaves_group_incomplete() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let time = FrameTime::from_utc(J2000);
    let mut times = StepTimes::default();
    let bad_site = Observer { latitude: f64::NAN, ..Observer::default() };
    begin_solar_system_frame(&mut simulation, &mut observer, time, bad_site, &mut times).unwrap();
    assert!(prepare_observer_inputs(&mut observer, &mut simulation, time, bad_site, &mut times).is_err());
    assert!(simulation.group.complete_key.is_none());
    assert!(simulation.group.has_been_invalidated);
    frame(&mut simulation, &mut observer, time, Observer::default());
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn inventory_reports_retained_sample_work_capacity() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    for day in [J2000, J2000 + 1.0] { frame(&mut simulation, &mut observer, FrameTime::from_utc(day), Observer::default()); }
    let snapshot = crate::state::collect_inventory("solar", &simulation);
    for name in ["planet_work", "moon_work", "orientation_work"] {
        assert!(snapshot.rows.iter().any(|row| row.path.ends_with(name) && row.used == Some(0) && row.reserved.is_some_and(|bytes| bytes > 0)), "missing retained scratch: {name}");
    }
}

#[test]
fn low_level_observer_replacement_cannot_reuse_an_old_completion_token() {
    let mut simulation = SimulationState::default();
    let mut observer = ObserverPreparationCache::default();
    let time = FrameTime::from_utc(J2000);
    let site = Observer::default();
    frame(&mut simulation, &mut observer, time, site);
    let generation = simulation.group.request_generation;
    let other_site = Observer { longitude: site.longitude + 0.1, ..site };
    crate::sky::prepare_cached_observer(&mut observer, &simulation, time, other_site).unwrap();
    assert!(observer.completed_observer(simulation.request_token()).is_none());
    frame(&mut simulation, &mut observer, time, site);
    assert!(simulation.group.request_generation > generation);
}
