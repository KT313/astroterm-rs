//! Stage order, provenance and view-independent intrinsic reuse across the explicit production boundaries.
use std::sync::Arc;
use astroterm::{astro::{J2000, Matrix3, Observer, Vector3, models::BodyState}, cache::CacheConfig,
    catalog::{Catalog, StarId, load_embedded_catalog}, model::{FrameTime, ObservedSky, SkyCatalog, SkyRegion},
    state::{SimulationState, ObserverPreparationCache, StarSelectionCache, StellarSimulationState, ObservationCache}, sky, timing::StepTimes};

fn catalog() -> Arc<SkyCatalog> {
    let mut source = load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(3);
    for (index, star) in source.stars.iter_mut().enumerate() {
        star.id = StarId(index as u32 + 1);
        star.hr = None;
        star.right_ascension = index as f64 * std::f64::consts::FRAC_PI_2;
        star.declination = 0.0;
        star.ra_motion = 0.0; star.ra_motion_cos_dec = 0.0; star.dec_motion = 0.0;
        star.space_motion = None; star.magnitude = 4.0;
    }
    Arc::new(sky::prepare_owned_catalog(Catalog::new(source.stars, Default::default(), vec![])).unwrap().catalog)
}
fn synthetic_observer() -> astroterm::model::ObserverState {
    let mut observer = sky::compose_observer_state(FrameTime::from_utc(J2000), Observer::default(), BodyState::default(), Matrix3::IDENTITY, BodyState::default(), false);
    observer.inertial_to_horizon = Matrix3::IDENTITY;
    observer
}
fn cone(x: f64, y: f64) -> SkyRegion { SkyRegion::Cone { center: Vector3 { x, y, z: 0.0 }, radius: 0.1 } }

#[test]
fn selection_changes_reuse_intrinsic_samples_and_only_calculate_new_rows() {
    let catalog = catalog();
    let observer = synthetic_observer();
    let mut selection = StarSelectionCache::default();
    let mut stars = StellarSimulationState::default();
    let mut times = StepTimes::default();
    sky::select_cached_stars(&mut selection, &catalog, &observer, 5.0, false, cone(1.0, 0.0), &mut times);
    let first = selection.stars().rows()[0].source_index;
    assert_eq!(selection.stars().rows().len(), 1);
    sky::simulate_stars(&mut stars, selection.stars(), observer.time.tt, &mut times);
    assert_eq!(stars.stellar_report(first).unwrap().stats.refreshes, 1);
    assert_eq!(stars.stats().refreshes, 2); // one per-star sample, one working-order result

    sky::select_cached_stars(&mut selection, &catalog, &observer, 5.0, false, SkyRegion::All, &mut times);
    sky::simulate_stars(&mut stars, selection.stars(), observer.time.tt, &mut times);
    assert_eq!(stars.stellar_report(first).unwrap().stats.refreshes, 1);
    for row in selection.stars().rows() { assert_eq!(stars.stellar_report(row.source_index).unwrap().stats.refreshes, 1); }
    assert_eq!(stars.stats().refreshes, 5); // three stars total and two assembled results

    sky::select_cached_stars(&mut selection, &catalog, &observer, 5.0, false, cone(0.0, 1.0), &mut times);
    sky::simulate_stars(&mut stars, selection.stars(), observer.time.tt, &mut times);
    let result = stars.results(selection.stars());
    assert_eq!(result.samples().len(), 1);
    assert!(result.samples()[0].0.y > 0.99);
    assert_eq!(stars.stellar_report(first).unwrap().stats.refreshes, 1);
}

#[test]
fn equal_generations_from_different_selections_cannot_reuse_other_rows() {
    let catalog = catalog(); let observer = synthetic_observer(); let mut times = StepTimes::default();
    let (mut a, mut b) = (StarSelectionCache::default(), StarSelectionCache::default());
    let mut stars = StellarSimulationState::default();
    sky::select_cached_stars(&mut a, &catalog, &observer, 5.0, false, cone(1.0, 0.0), &mut times);
    sky::select_cached_stars(&mut b, &catalog, &observer, 5.0, false, cone(0.0, 1.0), &mut times);
    sky::simulate_stars(&mut stars, a.stars(), observer.time.tt, &mut times);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stars.results(b.stars()))).is_err());
    sky::simulate_stars(&mut stars, b.stars(), observer.time.tt, &mut times);
    assert!(stars.results(b.stars()).samples()[0].0.y > 0.99);
}

#[test]
fn observation_reads_completed_results_without_sampling_or_propagating() {
    for config in [CacheConfig::default(), CacheConfig::disabled()] {
        let catalog = catalog();
        let mut simulation = SimulationState::default(); simulation.configure_cache(&config);
        let mut observer_cache = ObserverPreparationCache::new(config.clone());
        let mut selection = StarSelectionCache::new(config.clone());
        let mut stars = StellarSimulationState::new(config.clone());
        let mut corrections = ObservationCache::new(config);
        let mut output = ObservedSky::new(catalog.clone());
        let mut times = StepTimes::with_trace(true);
        let time = FrameTime::from_utc(J2000);
        times.measure_steps("Solar-system simulation", |times| sky::update_solar_system(&mut simulation, time, &[], times)).unwrap();
        let observer = times.measure_steps("Observer preparation", |times| sky::prepare_observer_inputs(&mut observer_cache, &mut simulation, time, Observer::default(), times)).unwrap();
        times.measure_steps("Star selection", |times| sky::select_cached_stars(&mut selection, &catalog, &observer, 5.0, true, SkyRegion::All, times));
        times.measure_steps("Stellar simulation", |times| sky::simulate_stars(&mut stars, selection.stars(), time.tt, times));
        let before = (simulation.refresh_counts, stars.reports(), stars.stats());
        times.measure_steps("Observation", |times| sky::observe_cached_sky(&mut corrections, stars.results(selection.stars()), observer_cache.bodies(&observer), &observer, 5.0, true, &mut output, times));
        assert_eq!(before, (simulation.refresh_counts, stars.reports(), stars.stats()));
        let trace = times.trace().unwrap();
        let parents: Vec<_> = trace.steps.iter().filter(|s| s.depth == 0).map(|s| s.name).collect();
        assert_eq!(parents, ["Solar-system simulation", "Observer preparation", "Star selection", "Stellar simulation", "Observation"]);
        let start = trace.steps.iter().position(|s| s.name == "Observation").unwrap();
        assert!(trace.steps[start..].iter().all(|s| !["Planet samples", "Lunar samples", "Orientation samples", "Body sampling", "Stellar motion", "Region filtering"].contains(&s.name)));
        for name in ["Body sampling", "Region filtering", "Brightness bounds", "Candidate validation", "Constellation endpoints", "Stellar motion", "Current brightness"] {
            assert!(!trace.steps.iter().find(|s| s.name == name).unwrap().details.is_empty(), "missing scoped details: {name}");
        }
    }
}
