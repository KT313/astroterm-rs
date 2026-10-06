use astroterm::astro::J2000;
use astroterm::cache::{CacheConfig, RefreshReason};
use astroterm::catalog::{Catalog, StarNames, load_embedded_catalog};
use astroterm::model::{ObservedSky, ProjectionViewport, View, RenderOptions};
use astroterm::projection::project_sky;
use astroterm::scene::draw_pixels;
use astroterm::sky::create_sky_from_catalog;
use astroterm::state::SceneCache;
use astroterm::timing::{StepTimes, BufferId, MemoryEvent, Operation};

fn fixture() -> ObservedSky {
    let mut catalog = load_embedded_catalog().unwrap();
    catalog.stars.truncate(3);
    create_sky_from_catalog(&Catalog::new(catalog.stars, StarNames::default(), vec![]))
}
fn options() -> RenderOptions {
    RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 5.0, label_threshold: 0.25, dynamic_names: false }
}
fn trace(enabled: bool) -> StepTimes {
    let mut times = StepTimes::with_trace(true);
    times.enable_memory_events(enabled);
    times
}
fn operations(times: &StepTimes, name: &str) -> Vec<MemoryEvent> {
    times.trace().unwrap().steps.iter().filter(|s| s.name == name).flat_map(|s| s.memory_events.iter().map(|e| e.event)).collect()
}
fn contains(events: &[MemoryEvent], buffer: BufferId, operation: Operation) -> bool {
    events.iter().any(|e| matches!(e, MemoryEvent::Operation { buffer: b, operation: o, .. } if *b == buffer && *o == operation))
}

#[test]
fn raster_hit_only_clears_candidate_and_copies_output_without_repainting() {
    let sky = fixture();
    let data = project_sky(&sky, &View::default(), ProjectionViewport { width: 16, height: 12 });
    let projected = data.view(&sky);
    let mut cache = SceneCache::default();
    let mut first = trace(true);
    let expected = draw_pixels(&mut cache, &projected, &options(), J2000, &mut first).unwrap();
    assert!(contains(&operations(&first, "Raster cache store"), BufferId::PixelCandidate, Operation::Move));
    assert!(contains(&operations(&first, "Raster cache store"), BufferId::PixelScene, Operation::Store { value_changed: true }));
    assert!(operations(&first, "Canvas initialization").iter().any(|e| matches!(e, MemoryEvent::Operation { operation: Operation::Build, .. })));

    let mut hit = trace(true);
    assert_eq!(draw_pixels(&mut cache, &projected, &options(), J2000, &mut hit).unwrap(), expected);
    assert!(contains(&operations(&hit, "Raster cache decision"), BufferId::PixelScene, Operation::Reuse));
    assert!(operations(&hit, "Canvas initialization").is_empty());
    assert!(operations(&hit, "Raster cache store").is_empty());
    let clears = operations(&hit, "Raster candidate clear");
    let MemoryEvent::Operation { before: Some(before), after: Some(after), .. } = clears[0] else { panic!("candidate shapes"); };
    assert_eq!(before.capacity, after.capacity);
    assert_eq!(after.len, Some(0));
    assert!(operations(&hit, "Raster output copy").iter().any(|e| matches!(e, MemoryEvent::Operation { buffer: BufferId::SkyImage, operation: Operation::Copy, logical_bytes: Some(bytes), .. } if *bytes == expected.len())));
}

#[test]
fn raster_invalidation_and_bypass_preserve_equal_result_store_outcomes() {
    let sky = fixture();
    let data = project_sky(&sky, &View::default(), ProjectionViewport { width: 16, height: 12 });
    let projected = data.view(&sky);
    let mut cache = SceneCache::default();
    let expected = draw_pixels(&mut cache, &projected, &options(), J2000, &mut trace(false)).unwrap();
    for reason in [RefreshReason::Invalidated, RefreshReason::Bypassed] {
        if reason == RefreshReason::Bypassed { cache.configure(&CacheConfig::disabled()); } else { cache.invalidate(); }
        let mut times = trace(true);
        assert_eq!(draw_pixels(&mut cache, &projected, &options(), J2000, &mut times).unwrap(), expected);
        let events = operations(&times, "Raster cache decision");
        assert!(contains(&events, BufferId::PixelScene, Operation::Refresh(reason)));
        assert!(!contains(&events, BufferId::PixelCandidate, Operation::Compare)); // both paths bypass key equality
        assert!(contains(&operations(&times, "Raster cache store"), BufferId::PixelScene, Operation::Store { value_changed: false }));
    }
}

#[test]
fn failed_raster_keeps_candidate_but_does_not_report_store_or_output_copy() {
    let sky = fixture();
    let data = project_sky(&sky, &View::default(), ProjectionViewport { width: 16, height: 12 });
    let mut projected = data.view(&sky);
    projected.viewport.width = usize::MAX;
    let mut cache = SceneCache::default();
    let mut failed = trace(true);
    assert!(draw_pixels(&mut cache, &projected, &options(), J2000, &mut failed).is_none());
    assert!(operations(&failed, "Raster cache store").is_empty());
    assert!(operations(&failed, "Raster output copy").is_empty());
    assert_eq!(cache.stats().refreshes, 0);
    projected.viewport.width = 16;
    let mut repaired = trace(true);
    assert!(draw_pixels(&mut cache, &projected, &options(), J2000, &mut repaired).is_some());
    assert!(contains(&operations(&repaired, "Raster cache key"), BufferId::PixelCandidate, Operation::Clear));
}

#[test]
fn normal_trace_without_memory_activation_leaves_rendering_events_empty() {
    let sky = fixture();
    let data = project_sky(&sky, &View::default(), ProjectionViewport { width: 16, height: 12 });
    let mut cache = SceneCache::default();
    let mut times = trace(false);
    draw_pixels(&mut cache, &data.view(&sky), &options(), J2000, &mut times).unwrap();
    assert!(times.trace().unwrap().steps.iter().all(|s| s.memory_events.is_empty()));
}

#[test]
fn changed_raster_inputs_report_dependency_comparison_and_changed_image() {
    let sky = fixture();
    let data = project_sky(&sky, &View::default(), ProjectionViewport { width: 16, height: 12 });
    let projected = data.view(&sky);
    let mut cache = SceneCache::default();
    let mut options = options();
    let first = draw_pixels(&mut cache, &projected, &options, J2000, &mut trace(false)).unwrap();
    options.grid = true;
    let mut changed = trace(true);
    let next = draw_pixels(&mut cache, &projected, &options, J2000, &mut changed).unwrap();
    assert_ne!(first, next);
    let decision = operations(&changed, "Raster cache decision");
    assert!(contains(&decision, BufferId::PixelCandidate, Operation::Compare));
    assert!(contains(&decision, BufferId::PixelScene, Operation::Refresh(RefreshReason::Dependencies)));
    assert!(contains(&operations(&changed, "Raster cache store"), BufferId::PixelScene, Operation::Store { value_changed: true }));
}
