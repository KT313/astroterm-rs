use crate::test_pipeline::{PipelineCache, prepare_stellar_catalog, prepare_cached_observer, prepare_cached_light_time, observe_cached_sky};
use crate::{model::{SkyCatalog, FrameTime, ObservedSky}, state::SimulationState, astro::Observer, timing::StepTimes};
use std::sync::Arc;
fn small_catalog() -> Arc<SkyCatalog> {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(6);
    Arc::new(crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(source.stars, Default::default(), vec![])).unwrap().catalog)
}

#[test]
fn catalog_replacement_retains_geometry_and_resets_catalog_state() {
    let first = small_catalog();
    let replacement = Arc::new((*first).clone()); // equal content, distinct identity must still reset catalog-indexed caches
    let mut storage = PipelineCache::default();
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(crate::astro::J2000);
    let mut times = StepTimes::default();
    prepare_stellar_catalog(&mut storage, first.clone(), &mut times);
    crate::sky::update_solar_system(&mut simulation, time, &[], &mut times).unwrap();
    let mut observer = prepare_cached_observer(&mut storage, &simulation, time, Observer::default()).unwrap();
    prepare_cached_light_time(&mut storage, &mut simulation, &mut observer, &mut times).unwrap();
    let mut sky = ObservedSky::new(first);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    let expected = sky.clone();
    let saved_observer = storage.observer.observer.clone();
    let saved_light_time = storage.observer.light_time.clone();
    assert!(storage.stars.prepared_classes.is_some());
    assert_eq!(storage.stars.regions.entries.iter().filter_map(|c| c.stored()).map(Vec::len).sum::<usize>(), 6);

    sky = ObservedSky::new(replacement.clone());
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    assert!(Arc::ptr_eq(storage.stars.catalog.as_ref().unwrap(), &replacement));
    assert!(storage.stars.prepared_classes.is_some()); // automatic replacement prepares the new region layout too
    assert_eq!(storage.observer.observer, saved_observer);
    assert_eq!(storage.observer.light_time, saved_light_time);
    assert_eq!(storage.stars.motion.stats.refreshes, 1);
    assert_eq!(storage.stars.region_stats.refreshes, crate::constants::SIMULATION_REGION_COUNT as u64);
    assert_eq!(sky.stars, expected.stars);
    assert_eq!(sky.planets, expected.planets);
    assert_eq!(sky.moon, expected.moon);
}

#[test]
fn missing_body_coverage_keeps_committed_body_cache_and_published_sky() {
    let catalog = small_catalog();
    let mut storage = PipelineCache::default();
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(crate::astro::J2000);
    let mut times = StepTimes::default();
    crate::sky::update_solar_system(&mut simulation, time, &[], &mut times).unwrap();
    let observer = crate::sky::prepare_observation(&mut simulation, time, Observer::default()).unwrap();
    let mut sky = ObservedSky::new(catalog);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    let previous_sky = sky.clone();
    let previous_bodies = storage.observer.bodies.clone();
    let mut missing = observer;
    missing.emission_tt[0] -= 10.0;
    assert!(observe_cached_sky(&mut storage, &simulation, &missing, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).is_err());
    assert_eq!(storage.observer.bodies.generation, previous_bodies.generation);
    assert_eq!(storage.observer.bodies.stats.refreshes, previous_bodies.stats.refreshes);
    let mut expected_failure = previous_bodies.clone();
    expected_failure.has_been_invalidated = true;
    expected_failure.stats.last_reason = Some(crate::cache::RefreshReason::Dependencies);
    assert!(storage.observer.bodies == expected_failure); // invalidated prior key/value remain owned and cannot be read until refreshed
    assert_eq!(sky, previous_sky);
    observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
    assert_eq!(sky, previous_sky);
    assert_eq!(storage.observer.bodies.generation, previous_bodies.generation);
}

#[test]
fn corrected_star_construction_preserves_rows_with_and_without_rejections() {
    use crate::cache::{Cache, CacheConfig};
    use crate::model::{SelectedStar, ObservedStar};
    use crate::state::{WorkingCache, MotionCache, EligibleCache};
    use crate::astro::Vector3;

    let mut output = ObservedSky::new(small_catalog());
    for flags in [vec![true, true, true], vec![true, false, true], vec![], vec![true, true, true]] {
        let mut working = WorkingCache::default();
        let mut motion = MotionCache::default();
        let mut eligible = EligibleCache::default();
        let mut corrections = Cache::default();
        let samples: Vec<_> = (0..flags.len()).map(|index| (Vector3 { x: index as f64, y: -2.0, z: 3.0 }, index as f64 + 1.5)).collect();
        working.store(0, 0.0, 0.0, (0..flags.len()).map(|source_index| SelectedStar { source_index, drawable: true }).collect());
        motion.store((Default::default(), 0, 0), 0.0, 0.0, (samples.clone(), 0));
        eligible.store((working.generation, motion.generation, 20.0), 0.0, 0.0, flags.clone());
        let expected: Vec<_> = flags.iter().enumerate().filter(|(_, drawable)| **drawable).map(|(index, _)| ObservedStar {
            source_index: index, drawable: true, position: samples[index].0, magnitude: samples[index].1,
        }).collect();

        for _ in 0..2 { // exercise both a new correction selection and its cache hit
            super::selection::update_correction_selection(&working, &eligible, &mut corrections, &motion, &CacheConfig::default(), 0.0, &mut output, &mut StepTimes::default());
            assert_eq!(output.stars, expected);
            assert_eq!(output.corrections.evaluated, flags.len());
            assert_eq!(output.corrections.skipped, flags.len() - expected.len());
        }
    }
}
