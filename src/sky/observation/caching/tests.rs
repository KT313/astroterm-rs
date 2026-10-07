use super::*;
use crate::sky::observe_cached_sky;
fn small_catalog() -> Arc<SkyCatalog> {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(6);
    Arc::new(crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(source.stars, Default::default(), vec![])).catalog)
}

#[test]
fn catalog_replacement_retains_geometry_and_resets_catalog_state() {
    let first = small_catalog();
    let replacement = Arc::new((*first).clone()); // equal content, distinct identity must still reset catalog-indexed caches
    let mut storage = ObservationCache::default();
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(crate::astro::J2000);
    let mut times = StepTimes::default();
    prepare_observation_catalog(&mut storage, first.clone(), &mut times);
    crate::sky::update_simulation(&mut simulation, time, &[], &mut times).unwrap();
    let mut observer = prepare_cached_observer(&mut storage, &simulation, time, Observer::default()).unwrap();
    prepare_cached_light_time(&mut storage, &mut simulation, &mut observer, &mut times).unwrap();
    let mut sky = ObservedSky::new(first);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    let expected = sky.clone();
    let saved_observer = storage.observer.clone();
    let saved_light_time = storage.light_time.clone();
    assert!(storage.prepared_classes.is_some());
    assert_eq!(storage.stellar.len(), 6);

    sky = ObservedSky::new(replacement.clone());
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    assert!(Arc::ptr_eq(storage.catalog.as_ref().unwrap(), &replacement));
    assert!(storage.prepared_classes.is_none()); // automatic replacement keeps the existing classify-on-demand policy
    assert_eq!(storage.observer, saved_observer);
    assert_eq!(storage.light_time, saved_light_time);
    assert_eq!(storage.motion.stats.refreshes, 1);
    assert_eq!(storage.stellar_stats.refreshes, 6);
    assert_eq!(sky.stars, expected.stars);
    assert_eq!(sky.planets, expected.planets);
    assert_eq!(sky.moon, expected.moon);
}

#[test]
fn missing_body_coverage_keeps_committed_body_cache_and_published_sky() {
    let catalog = small_catalog();
    let mut storage = ObservationCache::default();
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(crate::astro::J2000);
    let mut times = StepTimes::default();
    crate::sky::update_simulation(&mut simulation, time, &[], &mut times).unwrap();
    let observer = crate::sky::prepare_observation(&mut simulation, time, Observer::default()).unwrap();
    let mut sky = ObservedSky::new(catalog);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    let previous_sky = sky.clone();
    let previous_bodies = storage.bodies.clone();
    let mut missing = observer;
    missing.emission_tt[0] -= 10.0;
    assert!(observe_cached_sky(&mut storage, &simulation, &missing, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).is_err());
    assert_eq!(storage.bodies.generation, previous_bodies.generation);
    assert_eq!(storage.bodies.stats.refreshes, previous_bodies.stats.refreshes);
    let mut expected_failure = previous_bodies.clone();
    expected_failure.has_been_invalidated = true;
    expected_failure.stats.last_reason = Some(crate::cache::RefreshReason::Dependencies);
    assert!(storage.bodies == expected_failure); // invalidated prior key/value remain owned and cannot be read until refreshed
    assert_eq!(sky, previous_sky);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    assert_eq!(sky, previous_sky);
    assert_eq!(storage.bodies.generation, previous_bodies.generation);
}

#[test]
fn stellar_hold_bound_covers_forward_reverse_and_fast_motion() {
    let epoch = crate::astro::J2000;
    for speed in [0.0, 0.01, 10.0, 10000.0] {
        let motion = StellarMotion {
            u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w: Vector3 {
                x: 0.0,
                y: speed,
                z: 0.0,
            },
            distance_pc: None,
        };
        let sample = motion.evaluate(0.0, 5.0);
        let span = qualify_stellar_span(motion, sample, epoch, 5.0, 360.0);
        for fraction in [-1.0, -0.3, 0.0, 0.4, 1.0] {
            let direct = motion.evaluate(years_since_j2000(epoch + span * fraction / 86400.0), 5.0);
            let error = sample
                .direction
                .cross(direct.direction)
                .length()
                .atan2(sample.direction.dot(direct.direction));
            assert!(error.to_degrees() * 3600.0 <= 0.1);
            assert_eq!(direct.magnitude, sample.magnitude);
        }
    }
}
#[test]
fn variable_brightness_and_out_of_range_states_use_exact_epochs() {
    let motion = StellarMotion {
        u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
        w: Vector3 {
            x: -0.01,
            y: 0.001,
            z: 0.0,
        },
        distance_pc: Some(1.0),
    };
    let sample = motion.evaluate(0.0, 5.0);
    assert_eq!(
        qualify_stellar_span(motion, sample, crate::astro::J2000, 5.0, 360.0),
        0.0
    );
    assert_eq!(
        qualify_stellar_span(motion, sample, crate::astro::COMPUTATIONAL_INTERVAL.end_tt, 5.0, 360.0),
        0.0
    );
}
