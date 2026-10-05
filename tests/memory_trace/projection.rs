use super::*;
use astroterm::{
    astro::Vector3,
    cache::{CacheConfig, RefreshReason},
    model::{ObservedSky, projection::{ProjectionViewport, View}},
    projection::{borrow_projected, prepare_projection_catalog, project_cached_sky, project_sky},
    state::ProjectionCache,
    timing::TraceStep,
};

fn create_sky() -> ObservedSky {
    let mut parsed = astroterm::catalog::load_embedded_catalog().unwrap();
    parsed.stars.truncate(4);
    let mut sky = astroterm::sky::create_sky_from_catalog(&astroterm::catalog::Catalog::new(parsed.stars, parsed.names, vec![]));
    for (index, star) in sky.stars.iter_mut().enumerate() {
        star.drawable = true;
        star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        star.magnitude = index as f64;
    }
    sky
}

fn run(storage: &mut ProjectionCache, sky: &ObservedSky, memory: bool) -> StepTimes {
    let mut times = StepTimes::with_trace(true);
    times.enable_memory_events(memory);
    times.measure_steps("Projection", |times| project_cached_sky(storage, sky, &View::default(), ProjectionViewport { width: 80, height: 40 }, 0.0, times));
    times
}

fn find_step<'a>(times: &'a StepTimes, name: &str) -> &'a TraceStep {
    times.trace().unwrap().steps.iter().find(|step| step.name == name).unwrap()
}

fn operations(step: &TraceStep, buffer: BufferId) -> Vec<Operation> {
    step.memory_events.iter().filter_map(|record| match record.event {
        MemoryEvent::Operation { buffer: id, operation, .. } if id == buffer => Some(operation),
        _ => None,
    }).collect()
}

#[test]
fn projection_records_candidate_transfer_and_distinct_index_domains() {
    let mut sky = create_sky();
    sky.stars[1].drawable = false;
    sky.stars[3].position.z = -1.0;
    let mut storage = ProjectionCache::default();
    let times = run(&mut storage, &sky, true);
    let projection = find_step(&times, "Star projection");
    assert!(projection.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Borrow { buffer: BufferId::ObservedStars, access: Access::ReadOnly, shape: BufferShape { len: Some(4), domain: IndexDomain::Observed, .. } })));
    let key = find_step(&times, "Projection cache key");
    assert!(key.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { operation: Operation::Build, elements: Some(4), after: Some(BufferShape { domain: IndexDomain::Observed, .. }), .. })));
    let visible = find_step(&times, "Visible star calculation");
    assert!(visible.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::ProjectedCells, elements: Some(2), after: Some(BufferShape { domain: IndexDomain::Visible, .. }), .. })));
    let store = find_step(&times, "Projection cache store");
    assert!(store.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { operation: Operation::Move, before: Some(BufferShape { len: Some(4), .. }), after: Some(BufferShape { len: Some(0), capacity: Some(0), .. }), logical_bytes: Some(0), .. })));
    assert_eq!(operations(store, BufferId::ProjectedCells), [Operation::Compare, Operation::Store { value_changed: true }]);
    let order = find_step(&times, "Draw-order index extraction");
    assert!(order.memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::DrawOrder, elements: Some(2), after: Some(BufferShape { domain: IndexDomain::DrawOrder, .. }), .. })));
    assert_eq!(operations(find_step(&times, "Projection cache decision"), BufferId::ProjectionCandidate), []); // missing value skips key equality
    for step in ["Projection cache store", "Magnitude and ID sort"] {
        assert!(find_step(&times, step).memory_events.iter().any(|record| matches!(record.event,
            MemoryEvent::Operation { elements: None, logical_bytes: None, .. })));
    }
}

#[test]
fn projection_hits_clear_candidates_without_recalculating_or_changing_outputs() {
    let sky = create_sky();
    let mut storage = ProjectionCache::default();
    run(&mut storage, &sky, true);
    for _ in 0..2 {
        let times = run(&mut storage, &sky, true);
        assert!(times.trace().unwrap().steps.iter().all(|step| step.name != "Visible star calculation" && step.name != "Magnitude and ID sort"));
        for (name, buffer) in [("Projection candidate clear", BufferId::ProjectionCandidate), ("Draw-order candidate clear", BufferId::DrawOrderCandidate)] {
            let step = find_step(&times, name);
            let MemoryEvent::Operation { buffer: actual, operation: Operation::Clear, before: Some(before), after: Some(after), logical_bytes, .. } = step.memory_events[0].event else { panic!("clear event"); };
            assert_eq!(actual, buffer);
            assert_eq!(before.len, Some(4));
            assert_eq!(before.capacity, after.capacity);
            assert_eq!(after.len, Some(0));
            assert_eq!(logical_bytes, None);
        }
        assert_eq!(operations(find_step(&times, "Projection cache decision"), BufferId::ProjectedCells), [Operation::Reuse]);
        let compare = &find_step(&times, "Projection cache decision").memory_events[0];
        assert_eq!(compare.total_elements, None);
        assert_eq!(compare.total_logical_bytes, None);
        assert_eq!(operations(find_step(&times, "Body projection"), BufferId::ProjectedBodies), [Operation::Reuse]);
        let viewport = ProjectionViewport { width: 80, height: 40 };
        let reference = project_sky(&sky, &View::default(), viewport);
        assert_eq!(borrow_projected(&storage, &sky, &View::default(), viewport), reference.view(&sky));
    }
}

#[test]
fn projection_equal_refresh_changed_refresh_bypass_and_empty_work_are_distinguished() {
    let mut sky = create_sky();
    let mut storage = ProjectionCache::default();
    run(&mut storage, &sky, true);
    storage.invalidate_view();
    let equal = run(&mut storage, &sky, true);
    assert_eq!(operations(find_step(&equal, "Projection cache decision"), BufferId::ProjectedCells), [Operation::Refresh(RefreshReason::Invalidated)]);
    assert_eq!(operations(find_step(&equal, "Projection cache store"), BufferId::ProjectedCells), [Operation::Compare, Operation::Store { value_changed: false }]);
    sky.stars[0].drawable = false;
    let changed = run(&mut storage, &sky, true);
    assert_eq!(operations(find_step(&changed, "Projection cache decision"), BufferId::ProjectedCells), [Operation::Refresh(RefreshReason::Dependencies)]);
    assert_eq!(operations(find_step(&changed, "Projection cache store"), BufferId::ProjectedCells), [Operation::Compare, Operation::Store { value_changed: true }]);
    let mut disabled = ProjectionCache::new(CacheConfig::disabled());
    for _ in 0..2 {
        let bypass = run(&mut disabled, &sky, true);
        assert_eq!(operations(find_step(&bypass, "Projection cache decision"), BufferId::ProjectedCells), [Operation::Refresh(RefreshReason::Bypassed)]);
        assert_eq!(operations(find_step(&bypass, "Projection cache decision"), BufferId::ProjectionCandidate), []);
    }
    sky.stars.clear();
    let empty = run(&mut storage, &sky, true);
    assert!(find_step(&empty, "Visible star calculation").memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { elements: Some(0), logical_bytes: Some(0), .. })));
}

#[test]
fn projection_runtime_off_preserves_outputs_and_cache_stats_and_records_nothing() {
    let mut sky = create_sky();
    let mut enabled_cache = ProjectionCache::default();
    let mut disabled_cache = ProjectionCache::default();
    for iteration in 0..5 {
        if iteration == 2 { sky.stars[0].magnitude = 9.0; }
        if iteration == 3 { enabled_cache.invalidate_view(); disabled_cache.invalidate_view(); }
        run(&mut enabled_cache, &sky, true);
        let disabled = run(&mut disabled_cache, &sky, false);
        assert!(disabled.trace().unwrap().steps.iter().all(|step| step.memory_events.is_empty()));
        assert_eq!(enabled_cache.stats(), disabled_cache.stats());
        let viewport = ProjectionViewport { width: 80, height: 40 };
        assert_eq!(borrow_projected(&enabled_cache, &sky, &View::default(), viewport), borrow_projected(&disabled_cache, &sky, &View::default(), viewport));
    }
    let mut startup = enabled();
    prepare_projection_catalog(&mut enabled_cache, &sky.catalog, &mut startup);
    assert!(find_step(&startup, "Constellation topology").memory_events.iter().any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::PreparedEndpoints, operation: Operation::Copy, .. })));
}

#[test]
fn repeated_projection_calls_keep_cold_and_warm_events_on_their_own_invocations() {
    let sky = create_sky();
    let mut storage = ProjectionCache::default();
    let mut times = enabled();
    for _ in 0..2 {
        times.measure_steps("Projection", |times| project_cached_sky(&mut storage, &sky, &View::default(), ProjectionViewport { width: 80, height: 40 }, 0.0, times));
    }
    let decisions: Vec<_> = times.trace().unwrap().steps.iter().filter(|step| step.name == "Projection cache decision").collect();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0].depth, 2);
    assert_eq!(decisions[1].depth, 2);
    assert_eq!(operations(decisions[0], BufferId::ProjectedCells), [Operation::Refresh(RefreshReason::Missing)]);
    assert_eq!(operations(decisions[1], BufferId::ProjectedCells), [Operation::Reuse]);
    let bodies: Vec<_> = times.trace().unwrap().steps.iter().filter(|step| step.name == "Body projection").collect();
    assert_eq!(operations(bodies[0], BufferId::ProjectedBodies), [Operation::Refresh(RefreshReason::Missing), Operation::Build, Operation::Compare, Operation::Store { value_changed: true }]);
    assert_eq!(operations(bodies[1], BufferId::ProjectedBodies), [Operation::Reuse]);
}
