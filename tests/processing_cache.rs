//! Processing-cache reuse, invalidation and exact-reference comparisons through the production coordinators.
use astroterm::state::{ObservationCache, ProjectionCache, SceneCache, SimulationState};
use astroterm::astro::{J2000, Observer, Vector3};
use astroterm::cache::{CacheConfig, Group, GroupPolicy};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{
    ObservedSky, SkyCatalog, ProjectionViewport as Viewport, View, ViewCenter, RenderOptions, FrameTime, ModelFamily,
};
use astroterm::projection::project_sky;
use astroterm::scene::{draw_sky_scene, draw_pixel_sky};
use astroterm::sky::update_simulation;
use astroterm::timing::StepTimes;
use std::sync::Arc;

struct Pipeline {
    simulation: SimulationState,
    observation: ObservationCache,
    sky: ObservedSky,
    times: StepTimes,
}
impl Pipeline {
    fn new(catalog: Arc<SkyCatalog>, config: CacheConfig) -> Self {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(&config);
        let mut observation = ObservationCache::new(config.clone());
        if config.enabled {
            astroterm::sky::prepare_observation_catalog(&mut observation, catalog.clone(), &mut StepTimes::default());
        }
        Self {
            simulation,
            observation,
            sky: ObservedSky::new(catalog),
            times: StepTimes::default(),
        }
    }
    fn frame(&mut self, tt: f64, view: View, threshold: f64, refract: bool, site: Observer) {
        let time = FrameTime { tt, ut1: tt, utc: tt };
        self.times.begin_frame();
        self.simulation.begin_frame();
        update_simulation(&mut self.simulation, time, &[], &mut self.times).unwrap();
        let mut observer = astroterm::sky::prepare_cached_observer(&mut self.observation, &self.simulation, time, site).unwrap();
        astroterm::sky::prepare_cached_light_time(&mut self.observation, &mut self.simulation, &mut observer, &mut self.times)
            .unwrap();
        astroterm::sky::observe_cached_sky(&mut self.observation, &self.simulation,
                &observer,
                threshold,
                refract,
                astroterm::projection::select_view_region(&view),
                &mut self.sky,
                &mut self.times)
            .unwrap();
    }
}
fn catalog() -> Arc<SkyCatalog> {
    Arc::new(astroterm::sky::prepare_catalog(&load_embedded_catalog().unwrap()))
}
fn options() -> RenderOptions {
    RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 6.0,
        label_threshold: 0.5,
        dynamic_names: true,
    }
}
fn angle(a: Vector3, b: Vector3) -> f64 {
    a.cross(b).length().atan2(a.dot(b)).to_degrees() * 3600.0
}

#[test]
fn paused_frames_reuse_emission_samples_and_never_double_correct() {
    let mut p = Pipeline::new(catalog(), CacheConfig::default());
    let view = View::default();
    p.frame(J2000, view, 6.0, true, Observer::default());
    let counts = p.simulation.refresh_counts;
    let expected = p.sky.clone();
    for _ in 0..4 {
        p.frame(J2000, view, 6.0, true, Observer::default());
        assert_eq!(p.sky, expected);
        assert_eq!(p.simulation.refresh_counts, counts);
    }
    assert!(p.observation.stats().hits > 0);
    p.simulation.set_model_version(ModelFamily::Moon, 1);
    p.frame(J2000, view, 6.0, true, Observer::default());
    assert!(p.simulation.refresh_counts.moon > counts.moon);
    assert_eq!(p.simulation.refresh_counts.planets, counts.planets);
}

#[test]
fn bypass_recalculates_paused_frames_without_result_hits() {
    let mut p = Pipeline::new(catalog(), CacheConfig::disabled());
    p.frame(J2000, View::default(), 6.0, false, Observer::default());
    let expected = p.sky.clone();
    let counts = p.simulation.refresh_counts;
    p.frame(J2000, View::default(), 6.0, false, Observer::default());
    assert_eq!(p.sky, expected);
    assert!(p.simulation.refresh_counts.planets > counts.planets);
    assert!(p.simulation.refresh_counts.moon > counts.moon);
    assert_eq!(p.observation.stats().hits, 0);
    assert!(p.observation.stats().bypasses > 0);
}

#[test]
fn cached_pipeline_matches_reference_through_camera_time_and_site_changes() {
    let cat = catalog();
    let mut cached = Pipeline::new(cat.clone(), CacheConfig::default());
    let mut direct = Pipeline::new(cat, CacheConfig::disabled());
    let mut projection = ProjectionCache::new(CacheConfig::default());
    astroterm::projection::prepare_projection_catalog(&mut projection, &cached.sky.catalog, &mut StepTimes::default());
    let mut raster = SceneCache::default();
    astroterm::scene::prepare_scene_catalog(&mut raster, cached.sky.catalog.clone(), &mut StepTimes::default());
    let mut canvas = Canvas::new(45, 90);
    let mut maxima = [0.0_f64; 3];
    for (n, seconds) in [0.0, 1.0, 10.0, 31.0, 361.0, 5.0, -500.0, 86400.0, 0.0]
        .into_iter()
        .enumerate()
    {
        let view = View {
            fov_degrees: if n % 2 == 0 { 225.0 } else { 12.4 },
            center: ViewCenter::Facing {
                azimuth: n as f64 * 0.3,
                tilt: 0.4,
            },
            ..View::default()
        };
        let site = Observer {
            latitude: 0.4 + n as f64 * 0.01,
            longitude: 2.0,
        };
        let tt = J2000 + seconds / 86400.0;
        let threshold = if n % 2 == 0 { 6.0 } else { 4.0 };
        cached.frame(tt, view, threshold, n % 3 == 0, site);
        direct.frame(tt, view, threshold, n % 3 == 0, site);
        for star in &direct.sky.stars {
            let actual = cached
                .sky
                .stars
                .iter()
                .find(|s| s.source_index == star.source_index)
                .expect("no lost candidate or endpoint");
            assert_eq!(actual.magnitude, star.magnitude);
            assert_eq!(actual.drawable, star.drawable);
            maxima[0] = maxima[0].max(angle(actual.position, star.position));
            assert!(maxima[0] < 0.3);
        }
        for (a, b) in cached.sky.planets.iter().zip(&direct.sky.planets) {
            maxima[1] = maxima[1].max(angle(a.position, b.position));
            assert!(maxima[1] < 0.4);
        }
        maxima[2] = maxima[2].max(angle(cached.sky.moon.position, direct.sky.moon.position));
        assert!(maxima[2] < 0.6);
        let viewport = Viewport { height: 45, width: 90 };
        astroterm::projection::project_cached_sky(&mut projection, &cached.sky, &view, viewport, tt, &mut cached.times);
        let projected = astroterm::projection::borrow_projected(&projection, &cached.sky, &view, viewport);
        assert_eq!(projected, project_sky(&cached.sky, &view, viewport).view(&cached.sky));
        let mut expected = Canvas::new(45, 90);
        draw_sky_scene(&mut expected, &options(), &projected);
        astroterm::scene::draw_characters(&mut raster, &mut canvas, &projected, &options(), tt);
        assert_eq!(canvas, expected);
        astroterm::scene::draw_characters(&mut raster, &mut canvas, &projected, &options(), tt);
        assert_eq!(canvas, expected);
        let actual = astroterm::scene::draw_pixels(&mut raster, &projected, &options(), tt, &mut cached.times)
            .unwrap();
        let expected = draw_pixel_sky(&projected, &options(), &mut StepTimes::default()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            astroterm::scene::draw_pixels(&mut raster, &projected, &options(), tt, &mut cached.times)
                .unwrap(),
            expected
        );
    }
    println!("cached/direct maxima [stars, planets, Moon] arcseconds: {maxima:?}");
}

#[test]
fn paused_projection_and_raster_reuse_but_resize_and_options_invalidate() {
    let mut p = Pipeline::new(catalog(), CacheConfig::default());
    let view = View::default();
    p.frame(J2000, view, 6.0, false, Observer::default());
    let mut projection = ProjectionCache::new(CacheConfig::default());
    let mut raster = SceneCache::default();
    let viewport = Viewport { height: 70, width: 70 };
    astroterm::projection::project_cached_sky(&mut projection, &p.sky, &view, viewport, J2000, &mut p.times);
    let first = astroterm::projection::borrow_projected(&projection, &p.sky, &view, viewport);
    let image = astroterm::scene::draw_pixels(&mut raster, &first, &options(), J2000, &mut p.times).unwrap();
    let runs = projection.stats().refreshes;
    astroterm::projection::project_cached_sky(&mut projection, &p.sky, &view, viewport, J2000, &mut p.times);
    let second = astroterm::projection::borrow_projected(&projection, &p.sky, &view, viewport);
    assert_eq!(
        astroterm::scene::draw_pixels(&mut raster, &second, &options(), J2000, &mut p.times).unwrap(),
        image
    );
    assert_eq!(projection.stats().refreshes, runs);
    assert_eq!(raster.stats().hits, 1);
    astroterm::projection::project_cached_sky(&mut projection, &p.sky, &view, Viewport { height: 90, width: 90 }, J2000, &mut p.times);
    let resized = astroterm::projection::borrow_projected(&projection, &p.sky, &view, Viewport { height: 90, width: 90 });
    assert_eq!(
        astroterm::scene::draw_pixels(&mut raster, &resized, &options(), J2000, &mut p.times)
            .unwrap()
            .width(),
        90
    );
    let mut changed = options();
    changed.grid = true;
    astroterm::scene::draw_pixels(&mut raster, &resized, &changed, J2000, &mut p.times).unwrap();
    assert_eq!(raster.stats().refreshes, 3);
    raster.configure(&CacheConfig::disabled());
    for _ in 0..2 {
        astroterm::scene::draw_pixels(&mut raster, &resized, &changed, J2000, &mut p.times).unwrap();
    }
    assert_eq!(raster.stats().bypasses, 2);
}

#[test]
fn disabling_a_model_group_still_prepares_emission_coverage_each_frame() {
    let mut config = CacheConfig::default();
    config.groups.insert(
        Group::PlanetarySamples,
        GroupPolicy {
            enabled: false,
            max_age_seconds: None,
        },
    );
    let mut p = Pipeline::new(catalog(), config);
    for _ in 0..3 {
        p.frame(J2000, View::default(), 5.0, false, Observer::default());
    }
    assert!(p.simulation.refresh_counts.planets > 10);
}

#[test]
fn moving_distance_stars_preserve_magnitude_thresholds_and_order_within_ttl() {
    use astroterm::catalog::{Catalog, SpaceMotion};
    let mut source = load_embedded_catalog().unwrap();
    let mut a = source.stars.remove(0);
    let mut b = a.clone();
    a.id = astroterm::catalog::StarId(1);
    b.id = astroterm::catalog::StarId(2);
    for (star, rate) in [(&mut a, -1.0), (&mut b, 1.0)] {
        star.hr = None;
        star.name = None;
        star.magnitude = 5.0;
        star.right_ascension = 0.0;
        star.declination = 0.0;
        star.space_motion = Some(SpaceMotion {
            distance_pc: 1.0,
            position: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            velocity: Vector3 {
                x: rate,
                y: 0.5,
                z: 0.0,
            },
        });
    }
    let cat = Arc::new(astroterm::sky::prepare_owned_catalog(Catalog::new(
        vec![a, b],
        Default::default(),
        vec![],
    )));
    let mut cached = Pipeline::new(cat.clone(), CacheConfig::default());
    let mut direct = Pipeline::new(cat, CacheConfig::disabled());
    let view = View {
        fov_degrees: 360.0,
        projection: astroterm::model::ProjectionKind::Equidistant,
        ..View::default()
    };
    for seconds in [0.0, 10.0, -10.0, 100.0] {
        let tt = J2000 + seconds / 86400.0;
        cached.frame(tt, view, 5.0, false, Observer::default());
        direct.frame(tt, view, 5.0, false, Observer::default());
        assert_eq!(cached.sky.corrections.evaluated, 2);
        assert_eq!(cached.sky.stars.len(), if seconds == 0.0 { 2 } else { 1 });
        assert_eq!(cached.sky.corrections, direct.sky.corrections);
        for (a, b) in cached.sky.stars.iter().zip(&direct.sky.stars) {
            assert_eq!(
                (a.source_index, a.magnitude, a.drawable),
                (b.source_index, b.magnitude, b.drawable)
            );
            assert_eq!(
                cached.observation.stellar_report(a.source_index).unwrap().valid_seconds,
                0.0
            );
        }
    }
    // anchor both pipelines at the same epoch to isolate membership invalidation from sample-holding error
    let cat = cached.sky.catalog.clone();
    let mut cached = Pipeline::new(cat.clone(), CacheConfig::default());
    let mut direct = Pipeline::new(cat, CacheConfig::disabled());
    let tt = J2000 + 10.0 / 86400.0;
    for threshold in [5.0, 6.0, 5.0, 6.0] {
        cached.frame(tt, view, threshold, true, Observer::default());
        direct.frame(tt, view, threshold, true, Observer::default());
        assert_eq!(cached.sky.stars.len(), if threshold == 5.0 { 1 } else { 2 });
        for (a, b) in cached.sky.stars.iter().zip(&direct.sky.stars) {
            assert_eq!(
                (a.source_index, a.magnitude, a.drawable),
                (b.source_index, b.magnitude, b.drawable)
            );
            assert!(
                angle(a.position, b.position) < 1e-8,
                "threshold {threshold} id {:?}: {} arcsec",
                a.source_index,
                angle(a.position, b.position)
            );
        }
    }
}
