//! Observation events describe executed passes; a cache hit never fabricates a calculation or copy.
use super::cached;
use cached::PipelineCache;
use astroterm::astro::{J2000, Observer};
use astroterm::cache::{CacheConfig, RefreshReason};
use astroterm::model::{ObservedSky, SkyCatalog, SkyRegion, FrameTime};
use astroterm::sky;
use astroterm::state::{SimulationState};
use astroterm::timing::{StepTimes, BufferId, MemoryEvent, Operation};
use std::sync::Arc;

fn catalog(count: usize) -> Arc<SkyCatalog> {
    let mut source = astroterm::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(count);
    Arc::new(sky::prepare_owned_catalog(astroterm::catalog::Catalog::new(source.stars, source.names, vec![])).unwrap().catalog)
}
fn times(enabled: bool) -> StepTimes {
    let mut result = StepTimes::with_trace(true);
    result.enable_memory_events(enabled);
    result
}
fn frame(simulation: &mut SimulationState, cache: &mut PipelineCache, sky: &mut ObservedSky, threshold: f64, enabled: bool) -> StepTimes {
    let time = FrameTime::from_utc(J2000);
    let mut times = times(enabled);
    let observer = cached::prepare_frame(cache, simulation, time, Observer::default(), &mut times).unwrap();
    cached::observe_cached_sky(cache, simulation, &observer, threshold, true, SkyRegion::All, sky, &mut times).unwrap();
    times
}
fn operations(times: &StepTimes, buffer: BufferId) -> Vec<Operation> {
    times.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events).filter_map(|r| match r.event {
        MemoryEvent::Operation { buffer: id, operation, .. } if id == buffer => Some(operation),
        _ => None,
    }).collect()
}

#[test]
fn observation_hits_equal_refresh_and_bypass_report_executed_work_only() {
    let catalog = catalog(12);
    let mut simulation = SimulationState::default();
    let mut cache = PipelineCache::default();
    let mut sky = ObservedSky::new(catalog.clone());
    let first = frame(&mut simulation, &mut cache, &mut sky, 20.0, true);
    assert!(operations(&first, BufferId::RegionalBrightness).contains(&Operation::Build));
    assert!(!first.trace().unwrap().steps.iter().any(|step| ["Brightness output assembly", "Correction index selection", "Correction cache store"].contains(&step.name)));
    let subtraction = first.trace().unwrap().steps.iter().find(|step| step.name == "Observer subtraction").unwrap();
    assert!(subtraction.memory_events.iter().any(|event| matches!(event.event,
        MemoryEvent::Borrow { buffer: BufferId::BodySamples, access: astroterm::timing::Access::ReadOnly, shape }
            if shape.len == Some(sky.planets.len() + 1))));
    assert!(!subtraction.memory_events.iter().any(|event| matches!(event.event,
        MemoryEvent::Operation { buffer: BufferId::BodySamples, operation: Operation::Copy, .. })));

    let steps = &first.trace().unwrap().steps;
    let aberration = steps.iter().position(|step| step.name == "Aberration").unwrap();
    let children: Vec<_> = steps[aberration + 1..].iter().take_while(|step| step.depth > steps[aberration].depth).collect();
    assert!(children.iter().all(|step| !["Direction capture", "Direction restoration", "Direction cache store", "Apparent cache decision"].contains(&step.name)));
    assert!(!children.iter().flat_map(|step| &step.memory_events).any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::ObservedStars, operation: Operation::Write | Operation::Copy, .. })));
    let rotation = steps.iter().find(|step| step.name == "Horizon rotation calculation").unwrap();
    assert!(rotation.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Borrow { buffer: BufferId::BodyApparentDirections, access: astroterm::timing::Access::ReadOnly, shape } if shape.len == Some(sky.planets.len()))));
    assert!(!rotation.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Borrow { buffer: BufferId::RegionalApparent, .. }))); // stars are not rotated here; the view rotates them when read

    let expected = sky.clone();
    let second = frame(&mut simulation, &mut cache, &mut sky, 20.0, true);
    assert_eq!(sky, expected);
    assert_eq!(operations(&second, BufferId::RegionalBrightness), [Operation::Reuse]);
    assert!(operations(&second, BufferId::StellarSamples).contains(&Operation::Reuse));
    assert!(!second.trace().unwrap().steps.iter().any(|s| s.name == "Stellar batches"));
    assert!(!second.trace().unwrap().steps.iter().any(|s| s.name == "Direction capture"));
    let restoration: Vec<_> = second.trace().unwrap().steps.iter().filter(|s| s.name == "Direction restoration").collect();
    assert!(restoration.is_empty());
    for buffer in [BufferId::HorizontalDirections, BufferId::RefractedDirections] {
        let built = first.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events).find_map(|r| match r.event {
            MemoryEvent::Operation { buffer: id, operation: Operation::Build, elements, .. } if id == buffer => Some(elements),
            _ => None,
        }).unwrap();
        assert_eq!(built, Some(sky.planets.len() + 1)); // the bodies only; no star-sized direction buffer is built
        assert!(!operations(&first, buffer).contains(&Operation::Copy));
        assert!(operations(&second, buffer).contains(&Operation::Reuse));
        assert!(!operations(&second, buffer).contains(&Operation::Copy));
    }
    let changed_key = frame(&mut simulation, &mut cache, &mut sky, 21.0, true);
    assert!(operations(&changed_key, BufferId::RegionalBrightness).contains(&Operation::Build));
    cache.invalidate_view();
    let invalidated = frame(&mut simulation, &mut cache, &mut sky, 21.0, true);
    assert!(operations(&invalidated, BufferId::RegionSelection).contains(&Operation::Refresh(RefreshReason::Invalidated)));
    assert!(operations(&invalidated, BufferId::RegionSelection).contains(&Operation::Store { value_changed: false }));

    let mut bypass = PipelineCache::new(CacheConfig::disabled());
    let bypassed = frame(&mut simulation, &mut bypass, &mut ObservedSky::new(catalog), 20.0, true);
    assert!(operations(&bypassed, BufferId::RegionalBrightness).contains(&Operation::Build));
    assert!(bypass.selection.stats().bypasses > 0);
    assert!(!bypassed.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events).any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::ValidatedCandidates, .. }))); // generic candidate buffers are not used in cached selection
}

#[test]
fn stellar_batch_trace_is_bounded_and_counts_all_appended_samples() {
    let count = 2050;
    let mut simulation = SimulationState::default();
    let mut cache = PipelineCache::default();
    let mut sky = ObservedSky::new(catalog(count));
    let traced = frame(&mut simulation, &mut cache, &mut sky, 99.0, true);
    assert_eq!(sky.stars.len(), count);
    let trace = traced.trace().unwrap();
    assert!(!trace.steps.iter().any(|s| s.name == "Motion output assembly" || s.name == "Motion cache store"));
    let lookup = trace.steps.iter().find(|s| s.name == "Stellar region decisions").unwrap();
    assert!(lookup.memory_events.len() <= 8);
    let calculated = trace.steps.iter().find(|s| s.name == "Motion and magnitude calculation").unwrap();
    assert!(calculated.memory_aggregated);
    let appended = calculated.memory_events.iter().find(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::StellarOutputWork, operation: Operation::Append, .. })).unwrap();
    assert_eq!(appended.total_elements, Some(count));
    assert!(!trace.steps.iter().any(|s| s.name == "Region sample assembly"));
    assert!(!operations(&traced, BufferId::StellarScratch).contains(&Operation::Write));
    assert!(trace.steps.len() < 100); // independent of requested region/star count
    let clear = trace.steps.iter().find(|s| s.name == "Stellar scratch clear").unwrap();
    let MemoryEvent::Operation { before: Some(before), after: Some(after), .. } = clear.memory_events[0].event else { panic!("scratch clear boundaries"); };
    assert_eq!(before.capacity, after.capacity);
    assert_eq!(after.len, Some(0));
}

#[test]
fn observation_runtime_disabled_keeps_values_and_generations_identical() {
    let catalog = catalog(12);
    let mut simulation = SimulationState::default();
    let mut quiet_simulation = SimulationState::default();
    let mut a = PipelineCache::default();
    let mut b = PipelineCache::default();
    let mut sky_a = ObservedSky::new(catalog.clone());
    let mut sky_b = ObservedSky::new(catalog);
    for _ in 0..2 {
        frame(&mut simulation, &mut a, &mut sky_a, 20.0, true);
        let quiet = frame(&mut quiet_simulation, &mut b, &mut sky_b, 20.0, false);
        assert_eq!(sky_a, sky_b);
        assert_eq!(a.reports(), b.reports());
        assert!(quiet.trace().unwrap().steps.iter().all(|s| s.memory_events.is_empty()));
    }
}

#[test]
fn failed_body_refresh_records_request_but_no_commit_or_downstream_operations() {
    let mut simulation = SimulationState::default();
    let mut cache = PipelineCache::default();
    let mut sky = ObservedSky::new(catalog(12));
    frame(&mut simulation, &mut cache, &mut sky, 20.0, true);
    let expected = sky.clone();
    let time = FrameTime::from_utc(J2000);
    let mut observer = cached::prepare_cached_observer(&mut cache, &simulation, time, Observer::default()).unwrap();
    observer.emission_tt[0] -= 10.0;
    let mut traced = times(true);
    assert!(cached::observe_cached_sky(&mut cache, &simulation, &observer, 20.0, true, SkyRegion::All, &mut sky, &mut traced).is_err());
    assert_eq!(sky, expected);
    assert_eq!(operations(&traced, BufferId::BodySamples), [Operation::Refresh(RefreshReason::Dependencies)]);
    assert!(!operations(&traced, BufferId::StellarOutputWork).contains(&Operation::Append));
}

#[test]
fn preparation_and_model_reuse_keep_their_actual_buffer_identities() {
    let catalog = catalog(12);
    let mut cache = PipelineCache::default();
    let mut traced = times(true);
    cached::prepare_stellar_catalog(&mut cache, catalog, &mut traced);
    let preparation = traced.trace().unwrap().steps.iter().find(|s| s.name == "Stellar classifications").unwrap();
    assert!(preparation.memory_events.iter().any(|event| matches!(event.event,
        MemoryEvent::Operation { buffer: BufferId::CatalogClassifications, operation: Operation::Build, elements: Some(12), .. })));

    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(J2000);
    sky::update_solar_system(&mut simulation, time, &[], &mut traced).unwrap();
    sky::update_solar_system(&mut simulation, time, &[], &mut traced).unwrap();
    for (name, buffer, work) in [("Planet samples", BufferId::PlanetSamples, BufferId::PlanetSampleWork), ("Lunar samples", BufferId::LunarSamples, BufferId::LunarSampleWork), ("Orientation samples", BufferId::OrientationSamples, BufferId::OrientationSampleWork)] {
        let steps: Vec<_> = traced.trace().unwrap().steps.iter().filter(|s| s.name == name).collect();
        assert_eq!(steps.len(), 2);
        assert!(steps[0].memory_events.iter().any(|event| matches!(event.event,
            MemoryEvent::Operation { buffer: id, operation: Operation::Build, elements: Some(1), .. } if id == work)));
        assert!(steps[1].memory_events.iter().any(|event| matches!(event.event,
            MemoryEvent::Operation { buffer: id, operation: Operation::Reuse, elements: Some(1), .. } if id == buffer)));
        assert!(!steps[1].memory_events.iter().any(|event| matches!(event.event,
            MemoryEvent::Operation { operation: Operation::Copy | Operation::Build | Operation::Move, .. })));

    }
}

#[test]
fn empty_working_set_reports_zero_output_without_per_star_events() {
    let mut simulation = SimulationState::default();
    let mut cache = PipelineCache::default();
    let mut sky = ObservedSky::new(catalog(0));
    let traced = frame(&mut simulation, &mut cache, &mut sky, 20.0, true);
    assert!(sky.stars.is_empty());
    assert!(!traced.trace().unwrap().steps.iter().any(|s| s.name == "Stellar cache stores"));
    assert!(traced.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events).any(|event| matches!(event.event,
        MemoryEvent::Operation { buffer: BufferId::ObservedStars, operation: Operation::Build, elements: Some(0), logical_bytes: Some(0), .. })));
}
