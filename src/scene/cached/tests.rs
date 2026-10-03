use super::*;
use crate::{
    astro::{Horizontal, J2000},
    catalog::{Catalog, StarNames, load_embedded_catalog},
    scene::draw_sky_scene,
    sky::Sky,
};

fn options() -> RenderOptions {
    RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    }
}

fn fixture(name: &str, spectrum: [u8; 2], color_index: Option<f32>) -> Sky {
    let mut catalog = load_embedded_catalog().unwrap();
    catalog.stars.truncate(3);
    let mut names = StarNames::default();
    let named = names.insert(name);
    for (i, star) in catalog.stars.iter_mut().enumerate() {
        star.magnitude = 1.0 + i as f32;
        star.name = (i == 0).then_some(named);
        star.spectral_type = spectrum;
        star.color_index = color_index;
    }
    let mut sky = Sky::from_catalog(&Catalog::new(catalog.stars, names, vec![]));
    for (i, star) in sky.stars.iter_mut().enumerate() {
        star.position = Horizontal {
            azimuth: i as f64,
            altitude: 1.2,
        }
        .to_unit_vector();
    }
    let nadir = Horizontal {
        azimuth: 0.0,
        altitude: -1.5,
    }
    .to_unit_vector();
    for planet in &mut sky.planets {
        planet.position = nadir;
    }
    sky.moon.position = nadir;
    sky
}

fn project(sky: &Sky) -> ProjectedSky<'_> {
    project_sky(sky, &View::default(), Viewport { height: 40, width: 60 })
}

fn check_pixels(cache: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions) {
    let expected = draw_pixel_sky(sky, options, &mut StepTimes::default()).unwrap();
    let actual = cache
        .draw_pixels(sky, options, J2000, &mut StepTimes::default())
        .unwrap();
    assert_eq!(actual, expected);
    let hits = cache.pixels.stats.hits;
    assert_eq!(
        cache
            .draw_pixels(sky, options, J2000, &mut StepTimes::default())
            .unwrap(),
        expected
    );
    assert_eq!(cache.pixels.stats.hits, hits + 1);
}

fn check_characters(cache: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions, size: (usize, usize)) {
    let mut expected = Canvas::new(size.0, size.1);
    draw_sky_scene(&mut expected, options, sky);
    let mut actual = Canvas::new(size.0, size.1);
    cache.draw_characters(&mut actual, sky, options, J2000);
    assert_eq!(actual, expected);
    let hits = cache.characters.stats.hits;
    cache.draw_characters(&mut actual, sky, options, J2000);
    assert_eq!(actual, expected);
    assert_eq!(cache.characters.stats.hits, hits + 1);
}

#[test]
fn compact_pixel_keys_track_visual_metadata_across_catalogs_without_copying_names() {
    let mut cache = SceneCache::default();
    let mut last_pixels = None;
    for (name, spectrum, color_index) in [
        ("First", *b"B0", None),
        ("First", *b"M0", None),
        ("First", *b"  ", Some(-0.1)),
        ("First", *b"  ", Some(1.4)),
    ] {
        let sky = fixture(name, spectrum, color_index);
        let projected = project(&sky);
        let previous_refreshes = cache.pixels.stats.refreshes;
        check_pixels(&mut cache, &projected, &options());
        assert_eq!(cache.pixels.stats.refreshes, previous_refreshes + 1);
        if let Some(previous) = last_pixels {
            assert_ne!(previous, *cache.pixels.value());
        }
        last_pixels = Some(cache.pixels.value().clone());
    }

    // Names and celestial positions are not pixel-raster inputs once projected cells are fixed.
    let sky = fixture("Changed name", *b"  ", Some(1.4));
    let projected = project(&sky);
    let refreshes = cache.pixels.stats.refreshes;
    check_pixels(&mut cache, &projected, &options());
    assert_eq!(cache.pixels.stats.refreshes, refreshes);
    assert_eq!(std::mem::size_of::<keys::PixelStarKey>(), 24);
}

#[test]
fn compact_keys_track_geometry_order_threshold_and_calculated_magnitudes() {
    let mut sky = fixture("Alpha", *b"B0", None);
    let mut cache = SceneCache::default();
    let mut options = options();
    for phase in 0..7 {
        if phase == 1 {
            sky.stars[0].magnitude = -1.0;
        }
        if phase == 2 {
            options.magnitude_threshold = 2.0;
        }
        let mut projected = project(&sky);
        match phase {
            3 => projected.stars.reverse(),
            4 => projected.stars[0].cell = None,
            5 => projected.stars[0].cell = Some((5, 6)),
            6 => projected.stars.clear(),
            _ => {}
        }
        check_pixels(&mut cache, &projected, &options);
        check_characters(&mut cache, &projected, &options, (40, 60));
    }
}

#[test]
fn character_keys_capture_resolved_names_designations_options_and_canvas_size() {
    let mut cache = SceneCache::default();
    for name in ["First", "Other"] {
        let sky = fixture(name, *b"K0", None);
        let refreshes = cache.characters.stats.refreshes;
        check_characters(&mut cache, &project(&sky), &options(), (40, 60));
        assert_eq!(cache.characters.stats.refreshes, refreshes + 1);
    }
    for (index, name) in ["First", "Other", "α wide 星", "Last"].into_iter().enumerate() {
        // Name IDs have the same offset/length for First/Other; strings themselves must invalidate the key.
        let sky = fixture(name, *b"K0", None);
        let projected = project(&sky);
        let mut options = options();
        options.unicode = index % 2 == 0;
        options.color = index % 3 == 0;
        check_characters(&mut cache, &projected, &options, (40, 60));
        let before = cache.characters.stats.refreshes;
        check_characters(&mut cache, &projected, &options, (42, 63));
        assert_eq!(cache.characters.stats.refreshes, before + 1);
    }
}

#[test]
fn character_keys_track_dynamic_designation_changes_at_fixed_positions() {
    let mut cache = SceneCache::default();
    for designation in [
        crate::catalog::Designation::Hip(42),
        crate::catalog::Designation::Hip(99),
    ] {
        let mut source = load_embedded_catalog().unwrap();
        source.stars.truncate(1);
        source.stars[0].name = None;
        source.stars[0].magnitude = 1.0;
        source.stars[0].designation = Some(designation);
        let mut sky = Sky::from_catalog(&Catalog::new(source.stars, StarNames::default(), vec![]));
        sky.stars[0].position = Horizontal {
            azimuth: 0.0,
            altitude: 1.2,
        }
        .to_unit_vector();
        let mut projected = project(&sky);
        projected.planets.iter_mut().for_each(|planet| planet.cell = None);
        projected.moon.cell = None;
        let refreshes = cache.characters.stats.refreshes;
        check_characters(&mut cache, &projected, &options(), (40, 60));
        assert_eq!(cache.characters.stats.refreshes, refreshes + 1);
    }
}

#[test]
fn compact_keys_preserve_bodies_overlays_warnings_and_disabled_cache_behavior() {
    let sky = fixture("Alpha", *b"G0", None);
    let mut projected = project(&sky);
    let mut cache = SceneCache::default();
    let mut options = options();
    for phase in 0..10 {
        match phase {
            1 => projected.planets[0].cell = Some((12, 20)),
            2 => projected.moon.cell = Some((25, 30)),
            3 => projected.moon.illumination.illuminated_fraction = 0.7,
            4 => projected.moon.light_direction = Some(ScreenPoint { x: -1.0, y: 0.0 }),
            5 => options.grid = true,
            6 => {
                projected.facing = true;
                projected.horizon.push([(0, 0), (20, 20)]);
                projected.horizon_labels.push(((1, 1), "North"));
            }
            7 => projected.outside_accuracy_range = true,
            8 => cache.invalidate(),
            9 => options.dynamic_names = false,
            _ => {}
        }
        check_pixels(&mut cache, &projected, &options);
        check_characters(&mut cache, &projected, &options, (40, 60));
    }
    cache.configure(&CacheConfig::disabled());
    let expected = draw_pixel_sky(&projected, &options, &mut StepTimes::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            cache
                .draw_pixels(&projected, &options, J2000, &mut StepTimes::default())
                .unwrap(),
            expected
        );
    }
    assert_eq!(cache.pixels.stats.bypasses, 2);
}
