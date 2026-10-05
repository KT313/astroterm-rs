//! Run-history rotation must leave real cache results, geometry and raster output unchanged.
use astroterm::astro::{J2000, Observer};
use astroterm::cache::CacheConfig;
use astroterm::model::{ObservedSky, SkyCatalog};
use astroterm::model::projection::{ProjectionViewport, View};
use astroterm::model::rendering::RenderOptions;
use astroterm::model::simulation::FrameTime;
use astroterm::projection::{borrow_projected, project_cached_sky, select_view_region};
use astroterm::scene::cached::draw_pixels;
use astroterm::sky;
use astroterm::state::{ObservationCache, ProjectionCache, SceneCache, SimulationState};
use astroterm::timing::StepTimes;
use std::sync::Arc;

#[test]
fn continuous_history_preserves_calculations_and_cache_behavior() {
    let mut source = astroterm::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(24);
    let catalog = Arc::new(sky::prepare_owned_catalog(astroterm::catalog::Catalog::new(source.stars, Default::default(), vec![])));
    for config in [CacheConfig::default(), CacheConfig::disabled()] {
        let plain = run_frames(catalog.clone(), &config, false);
        let traced = run_frames(catalog.clone(), &config, true);
        assert_eq!(plain, traced);
    }
}

#[derive(Debug, PartialEq)]
struct FrameResult {
    sky: ObservedSky,
    image: image::RgbaImage,
    observation: Vec<astroterm::cache::CacheReport>,
    projection: astroterm::cache::CacheStats,
    scene: astroterm::cache::CacheStats,
}

fn run_frames(catalog: Arc<SkyCatalog>, config: &CacheConfig, diagnostics: bool) -> Vec<FrameResult> {
    let mut times = StepTimes::default();
    if diagnostics { times.enable_memory_run(config.enabled); }
    let mut sky = ObservedSky::new(catalog);
    let mut simulation = SimulationState::default();
    simulation.configure_cache(config);
    let mut observation = ObservationCache::new(config.clone());
    let mut projection = ProjectionCache::new(config.clone());
    let mut scene = SceneCache::default();
    scene.configure(config);
    let options = RenderOptions { unicode: true, braille: false, color: true, constellations: false, grid: false, magnitude_threshold: 8.0, label_threshold: 0.25, dynamic_names: false };
    let mut results = Vec::new();
    for (index, date) in [J2000, J2000, J2000 + 0.001].into_iter().enumerate() {
        times.begin_frame();
        if diagnostics { times.begin_memory_frame(); }
        let time = FrameTime::from_utc(date);
        if diagnostics { times.set_memory_frame_time(time.utc, time.tt); }
        simulation.begin_frame();
        sky::update_simulation(&mut simulation, time, &[], &mut times).unwrap();
        let mut observer = sky::prepare_cached_observer(&mut observation, &simulation, time, Observer::default()).unwrap();
        sky::prepare_cached_light_time(&mut observation, &mut simulation, &mut observer, &mut times).unwrap();
        let view = View::default();
        sky::observe_cached_sky(&mut observation, &simulation, &observer, 8.0, true, select_view_region(&view), &mut sky, &mut times).unwrap();
        let viewport = ProjectionViewport { width: if index == 2 { 48 } else { 32 }, height: 32 };
        project_cached_sky(&mut projection, &sky, &view, viewport, time.tt, &mut times);
        let projected = borrow_projected(&projection, &sky, &view, viewport);
        let image = draw_pixels(&mut scene, &projected, &options, time.tt, &mut times).unwrap();
        results.push(FrameResult { sky: sky.clone(), image, observation: observation.reports(), projection: projection.stats(), scene: scene.stats() });
        if diagnostics {
            times.complete_memory_frame(0.01);
            assert_eq!(times.memory_run().unwrap().completed_frames, index as u64 + 1);
            assert_eq!(times.memory_run().unwrap().latest.as_ref().unwrap().utc, Some(date));
        }
    }
    results
}
