//! Trusted rendering keeps exact pixels while skipping all per-star raster preparation on a paused hit.
use std::sync::Arc;
use astroterm::{astro::{J2000, Observer}, cache::{CacheConfig, Group, GroupPolicy},
    model::{FrameTime, ObservedSky, SkyCatalog, SkyRegion, View, ProjectionViewport, RenderOptions, RenderProjection},
    state::{SimulationState, ObserverPreparationCache, StarSelectionCache, StellarSimulationState, ObservationCache, ProjectionCache, SceneCache},
    sky, projection, scene, timing::StepTimes};

struct Run {
    catalog: Arc<SkyCatalog>, solar: SimulationState, observer: ObserverPreparationCache,
    selection: StarSelectionCache, stellar: StellarSimulationState, observation: ObservationCache,
    projection: ProjectionCache, summary: ObservedSky,
}
impl Run {
    fn new(color: f32) -> Self {
        let mut source = astroterm::catalog::load_embedded_catalog().unwrap();
        source.stars.retain(|star| star.has_data); source.stars.truncate(32);
        source.constellations.clear();
        for (index, star) in source.stars.iter_mut().enumerate() {
            star.space_motion = None; star.ra_motion = 0.0; star.ra_motion_cos_dec = 0.0; star.dec_motion = 0.0;
            star.right_ascension = index as f64 * 0.2; star.declination = 0.2;
            star.magnitude = (index % 8) as f64; star.color_index = Some(color);
        }
        let catalog = Arc::new(sky::prepare_owned_catalog(source).unwrap().catalog);
        Self { summary: ObservedSky::new(catalog.clone()), catalog, solar: SimulationState::default(),
            observer: ObserverPreparationCache::default(), selection: StarSelectionCache::default(),
            stellar: StellarSimulationState::default(), observation: ObservationCache::default(), projection: ProjectionCache::default() }
    }

    fn project(&mut self, utc: f64, threshold: f64, view: View, viewport: ProjectionViewport, region: SkyRegion) -> RenderProjection<'_> {
        let mut times = StepTimes::default(); let time = FrameTime::from_utc(utc); let site = Observer::default();
        sky::begin_solar_system_frame(&mut self.solar, &mut self.observer, time, site, &mut times).unwrap();
        let observer = sky::prepare_observer_inputs(&mut self.observer, &mut self.solar, time, site, &mut times).unwrap();
        sky::select_cached_stars(&mut self.selection, &self.catalog, &observer, threshold, false, region, &mut times);
        sky::simulate_stars(&mut self.stellar, self.selection.stars(), time.tt, &mut times);
        let observed = sky::observe_cached_regions(&mut self.observation, self.stellar.results(self.selection.stars()), self.observer.bodies(&observer), &observer, threshold, false, &mut self.summary, &mut times);
        projection::project_cached_regions(&mut self.projection, observed, &view, viewport, time.tt, &mut times);
        let observed = self.observation.regional_view(self.stellar.results(self.selection.stars()), &self.summary); // same reborrow used after production table logging
        projection::borrow_render_projection(&self.projection, observed, &view, viewport)
    }
}

fn options() -> RenderOptions {
    RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 8.0, dynamic_names: true }
}
fn viewport() -> ProjectionViewport { ProjectionViewport { width: 96, height: 80 } }
fn did(times: &StepTimes, name: &str) -> bool { times.trace().unwrap().steps.iter().any(|step| step.name == name) }
fn check(cache: &mut SceneCache, projected: &RenderProjection<'_>, options: &RenderOptions, redraw: bool) {
    let reference = scene::draw_pixel_sky(projected.sky(), options, &mut StepTimes::default()).unwrap();
    let mut times = StepTimes::with_trace(true);
    assert_eq!(scene::draw_prepared_pixels(cache, projected, options, J2000, &mut times).unwrap(), &reference);
    assert_eq!(did(&times, "Raster stars"), redraw);
    assert!(!did(&times, "Raster cache key"));
}

#[test]
fn paused_production_skips_star_input_work_and_keeps_image_allocation() {
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    check(&mut cache, &projected, &options(), true);
    let pointer = cache.pixel_image().as_ptr(); let generation = cache.pixel_generation();
    for _ in 0..3 {
        let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
        check(&mut cache, &projected, &options(), false);
        assert_eq!(cache.pixel_image().as_ptr(), pointer);
        assert_eq!(cache.pixel_generation(), generation);
    }
}

#[test]
fn changes_to_geometry_options_source_and_membership_refresh_the_image() {
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let zoom = View { fov_degrees: 90.0, ..View::default() };
    for (date, threshold, view, size, region) in [
        (J2000, 8.0, View::default(), viewport(), SkyRegion::All),
        (J2000, 8.0, zoom, viewport(), SkyRegion::All),
        (J2000, 8.0, zoom, ProjectionViewport { width: 72, height: 60 }, SkyRegion::All),
        (J2000 + 1.0, 8.0, zoom, viewport(), SkyRegion::All),
        (J2000 + 1.0, 3.0, zoom, viewport(), SkyRegion::All),
        (J2000, 8.0, View::default(), viewport(), SkyRegion::Cone { center: astroterm::astro::Vector3 { x: 1.0, y: 0.0, z: 0.0 }, radius: 0.2 }),
        (J2000, 8.0, View::default(), viewport(), SkyRegion::All),
    ] {
        let projected = run.project(date, threshold, view, size, region);
        check(&mut cache, &projected, &options(), true);
        check(&mut cache, &projected, &options(), false);
    }
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    check(&mut cache, &projected, &RenderOptions { grid: true, ..options() }, true);
    check(&mut cache, &projected, &options(), true); // isolate the following source change from option changes
    let mut other = Run::new(1.7); // same projection owner, replaced observation/catalog with equal local generations
    other.projection = std::mem::take(&mut run.projection);
    let projected = other.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    check(&mut cache, &projected, &options(), true);
}

#[test]
fn exact_and_production_modes_never_share_unverified_keys() {
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    check(&mut cache, &projected, &options(), true);
    scene::draw_pixels(&mut cache, projected.sky(), &options(), J2000, &mut StepTimes::default()).unwrap();
    check(&mut cache, &projected, &options(), true);
    check(&mut cache, &projected, &options(), false);
    scene::draw_pixels(&mut cache, projected.sky(), &options(), J2000, &mut StepTimes::default()).unwrap();
    let generation = cache.pixel_generation();
    check(&mut cache, &projected, &options(), true);
    assert_eq!(cache.pixel_generation(), generation, "equal images preserve result generation across key modes");
}

#[test]
fn explicit_invalidation_and_bypass_keep_the_same_pixels() {
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    check(&mut cache, &projected, &options(), true);
    let generation = cache.pixel_generation();
    cache.invalidate(); check(&mut cache, &projected, &options(), true);
    for config in [CacheConfig::disabled(), CacheConfig { groups: [(Group::Raster, GroupPolicy { enabled: false, ..Default::default() })].into(), ..Default::default() }] {
        cache.configure(&config);
        for _ in 0..2 { check(&mut cache, &projected, &options(), true); }
    }
    assert_eq!(cache.pixel_generation(), generation);
}

#[test]
fn invalidated_projection_cannot_issue_trusted_rendering_input() {
    let mut run = Run::new(0.2);
    run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    run.projection.invalidate_view();
    let observed = run.observation.regional_view(run.stellar.results(run.selection.stars()), &run.summary);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| projection::borrow_render_projection(&run.projection, observed, &View::default(), viewport())));
    assert!(result.is_err());
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn hit_diagnostics_do_not_report_star_input_work_or_geometry_copies() {
    use astroterm::timing::{BufferId, MemoryEvent, Operation};
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    let mut first = StepTimes::with_trace(true); first.enable_memory_events(true);
    scene::draw_prepared_pixels(&mut cache, &projected, &options(), J2000, &mut first).unwrap();
    assert!(first.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events).any(|e| matches!(e.event,
        MemoryEvent::Operation { buffer: BufferId::StarLayer, operation: Operation::Build, .. })));
    let mut hit = StepTimes::with_trace(true); hit.enable_memory_events(true);
    scene::draw_prepared_pixels(&mut cache, &projected, &options(), J2000, &mut hit).unwrap();
    for event in hit.trace().unwrap().steps.iter().flat_map(|s| &s.memory_events) {
        if let MemoryEvent::Operation { buffer, operation, .. } = event.event {
            assert_ne!(buffer, BufferId::StarLayer);
            assert_ne!(operation, Operation::Copy);
        }
    }
    assert!(!did(&hit, "Raster stars"));
}

#[test]
fn empty_requested_regions_are_part_of_the_dependency_identity() {
    let mut run = Run::new(0.2); let mut cache = SceneCache::default();
    let empty = astroterm::catalog::Catalog::new(vec![], Default::default(), vec![]);
    run.catalog = Arc::new(sky::prepare_owned_catalog(empty).unwrap().catalog);
    run.summary = ObservedSky::new(run.catalog.clone());
    let projected = run.project(J2000, 8.0, View::default(), viewport(), SkyRegion::All);
    assert!(projected.sky().stars.is_empty());
    check(&mut cache, &projected, &options(), true);
    check(&mut cache, &projected, &options(), false);
    let cone = SkyRegion::Cone { center: astroterm::astro::Vector3 { x: 1.0, y: 0.0, z: 0.0 }, radius: 0.2 };
    let projected = run.project(J2000, 8.0, View::default(), viewport(), cone);
    check(&mut cache, &projected, &options(), true);
    check(&mut cache, &projected, &options(), false);
}
