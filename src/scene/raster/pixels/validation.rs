//! Scene-level checks for the premultiplied star layer, cache reuse, labels and unrelated drawing layers.
use super::*;
use crate::astro::Vector3;
use crate::catalog::load_embedded_catalog;
use crate::model::{Sky, ProjectionData, ProjectionViewport as Viewport, View};
use crate::projection::project_sky;
use crate::state::SceneCache;

fn options() -> RenderOptions {
    RenderOptions { unicode: true, braille: true, color: true, constellations: true, grid: false,
        magnitude_threshold: 20.0, dynamic_names: true }
}
fn fixture(width: usize, height: usize) -> (Sky, ProjectionData) {
    let mut source = load_embedded_catalog().unwrap();
    source.stars.truncate(8);
    source.constellations.clear();
    for (i, star) in source.stars.iter_mut().enumerate() { star.magnitude = i as f64; }
    let mut sky = crate::sky::create_sky_from_catalog(&source).unwrap();
    for star in &mut sky.stars { star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 }; }
    for planet in &mut sky.planets { planet.position = Vector3 { x: 0.0, y: 0.0, z: -1.0 }; }
    sky.moon.position = Vector3 { x: 0.0, y: 0.0, z: -1.0 };
    let data = project_sky(&sky, &View::default(), Viewport { width, height });
    (sky, data)
}

#[test]
fn cached_fresh_and_bypassed_images_match_at_tiny_and_large_dimensions() {
    let mut cache = SceneCache::default();
    for (width, height) in [(1, 1), (2, 2), (64, 48), (4097, 3), (3, 4097)] {
        let (sky, data) = fixture(width, height);
        let projected = data.view(&sky);
        let expected = draw_pixel_sky(&projected, &options(), &mut StepTimes::default()).unwrap();
        for _ in 0..2 {
            assert_eq!(crate::scene::draw_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()), Some(&expected));
        }
        assert!(expected.pixels().all(|p| p[3] == 255));
        assert_eq!(cache.star_layer.len(), width * height);
        let allocation = cache.star_layer.as_ptr();
        cache.invalidate();
        assert_eq!(crate::scene::draw_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()), Some(&expected));
        assert_eq!(cache.star_layer.as_ptr(), allocation);
        cache.configure(&crate::cache::CacheConfig::disabled());
        assert_eq!(crate::scene::draw_pixels(&mut cache, &projected, &options(), 0.0, &mut StepTimes::default()), Some(&expected));
        cache.configure(&crate::cache::CacheConfig::default());
    }
}

#[test]
fn zoom_refreshes_brightness_even_when_projected_star_inputs_do_not_move() {
    use crate::constants::{STAR_OPACITY_REFERENCE_MAGNITUDE, STAR_OPACITY_MAGNITUDE_SCALE,
        STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES, STAR_BRIGHTNESS_ZOOM_POWER};
    for projection in [crate::model::ProjectionKind::Stereographic, crate::model::ProjectionKind::Equidistant] {
        let (mut sky, _) = fixture(32, 32);
        for star in &mut sky.stars { star.position.z = -1.0; }
        sky.stars[0].position.z = 1.0;
        sky.stars[0].magnitude = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;
        let options = RenderOptions { magnitude_threshold: f64::INFINITY, ..options() };
        let original_stars = sky.stars.clone();
        let mut cache = SceneCache::default();
        let mut baseline = None;
        let mut initial_inputs = None;
        for fov in [STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES, STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES / 2.0,
            STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES] {
            let view = View { projection, fov_degrees: fov, ..View::default() };
            let data = project_sky(&sky, &view, Viewport { width: 32, height: 32 });
            assert_eq!(data.fov_degrees, fov);
            let projected = data.view(&sky);
            assert_eq!(projected.fov_degrees, fov);
            let refreshes = cache.stats().refreshes;
            let actual = crate::scene::draw_pixels(&mut cache, &projected, &options, 0.0, &mut StepTimes::default()).unwrap().clone();
            assert_eq!(cache.stats().refreshes, refreshes + 1);
            let inputs = &cache.pixels.key().unwrap().stars;
            if let Some(expected) = &initial_inputs { assert!(inputs == expected); }
            else { initial_inputs = Some(inputs.clone()); }
            assert_eq!(actual, draw_pixel_sky(&projected, &options, &mut StepTimes::default()).unwrap());
            if fov == STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES {
                if let Some(expected) = &baseline { assert_eq!(&actual, expected); }
                else { baseline = Some(actual.clone()); }
            } else if STAR_BRIGHTNESS_ZOOM_POWER > 0.0 {
                assert_ne!(&actual, baseline.as_ref().unwrap());
                let (y, x) = projected.stars.get(0).cell.unwrap();
                assert!(actual[(x as u32, y as u32)][2] > baseline.as_ref().unwrap()[(x as u32, y as u32)][2]);
            }
            let hits = cache.stats().hits;
            assert_eq!(crate::scene::draw_pixels(&mut cache, &projected, &options, 0.0, &mut StepTimes::default()), Some(&actual));
            assert_eq!(cache.stats().hits, hits + 1);
        }
        assert_eq!(sky.stars, original_stars); // zoom changes display opacity, never physical magnitudes or positions
    }
}

#[test]
fn edge_stars_are_omitted_before_drawing_and_do_not_use_label_slots() {
    let (sky, mut data) = fixture(64, 48);
    let brightest = *data.order.last().unwrap();
    let second = data.order[data.order.len()-2];
    data.stars[brightest].1 = (0, 20);
    data.stars[second].1 = (20, 0);
    let projected = data.view(&sky);
    let selected = select_pixel_star_labels(&options(), &projected).collect::<Vec<_>>();
    assert_eq!(selected, vec![1, 2, 3, 4, 5]); // the last two entries are brighter but have no complete footprint
    let mut times = StepTimes::with_trace(true);
    let mut cache = SceneCache::default();
    crate::scene::draw_pixels(&mut cache, &projected, &options(), 0.0, &mut times).unwrap();
    let trace = times.trace().unwrap();
    let stars = trace.steps.iter().find(|s| s.name == "Raster stars").unwrap();
    assert!(stars.details[0].contains("omitted edge stars=2; submitted stars=6"));
    let names: Vec<_> = trace.steps.iter().map(|s| s.name).collect();
    for pair in [ ["Star layer initialization", "Raster stars"], ["Star brightness preparation", "Raster stars"],
        ["Raster stars", "Canvas initialization"], ["Raster horizon", "Star layer composition"],
        ["Star layer composition", "Raster planets"] ] {
        assert!(names.iter().position(|&n| n == pair[0]).unwrap() < names.iter().position(|&n| n == pair[1]).unwrap());
    }
    let labels_off = RenderOptions { dynamic_names: false, ..options() };
    assert_eq!(select_pixel_star_labels(&labels_off, &projected).len(), 0);
}

#[test]
fn empty_star_layer_does_not_change_horizon_constellations_planets_moon_or_grid() {
    let (sky, mut data) = fixture(64, 48);
    data.order.clear();
    data.facing = true;
    data.horizon = vec![[(12, 0), (12, 63)]];
    data.planets[0].cell = Some((20, 20));
    data.moon.cell = Some((30, 40));
    data.constellations.push(crate::model::ProjectedConstellation { maximum_magnitude: 2.0,
        arcs: vec![crate::model::ProjectedArc { start: (3, 4), end: (15, 50), points: vec![(3, 4), (15, 50)],
            includes_start: true, includes_end: true }] });
    let options = RenderOptions { grid: true, ..options() };
    for facing in [true, false] {
        data.facing = facing;
        let projected = data.view(&sky);
        let actual = draw_pixel_sky(&projected, &options, &mut StepTimes::default()).unwrap();
        let mut expected = initialize_pixel_canvas(projected.viewport, &mut Vec::new()).unwrap();
        draw_pixel_horizon(&mut expected, &projected);
        draw_pixel_constellations(&mut expected, &projected, &options);
        draw_pixel_planets(&mut expected, &projected);
        draw_pixel_moon(&mut expected, &projected);
        draw_pixel_grid(&mut expected, &projected, &options);
        assert_eq!(actual.as_raw(), expected.data());
    }
}

#[test]
fn blended_layer_table_reports_straight_colors_and_owned_capacity() {
    use crate::state::Tables;
    let (sky, data) = fixture(16, 16);
    let mut cache = SceneCache::default();
    crate::scene::draw_pixels(&mut cache, &data.view(&sky), &options(), 0.0, &mut StepTimes::default()).unwrap();
    let mut found = false;
    cache.visit_tables("scene", &mut |path, table, _| {
        if path == "scene.star_layer" {
            found = true;
            assert_eq!(table.rows(), 256);
            assert_eq!(table.bytes().used, Some(256 * 16));
            assert!(table.bytes().reserved.unwrap() >= 256 * 16);
            assert_eq!(table.columns().iter().map(|c| c.name).collect::<Vec<_>>(), ["rgb", "opacity"]);
        }
    });
    assert!(found);
    #[cfg(feature = "memory-diagnostics")]
    {
        let inventory = crate::state::collect_inventory("scene", &cache);
        let row = inventory.rows.iter().find(|r| r.kind == crate::cache::Kind::Heap && r.path.ends_with(".star_layer")).unwrap();
        assert_eq!(row.used, Some(256 * 16));
        assert!(row.reserved.unwrap() >= 256 * 16);
    }
}
