use crate::scene::{draw_pixels, draw_characters};
#[cfg(feature = "memory-diagnostics")]
use crate::scene::draw_characters_with_times;
use crate::model::{View, ProjectionViewport as Viewport, ScreenPoint};
use crate::projection::project_sky;
use super::*;
use crate::astro::{Horizontal, J2000};
use crate::catalog::{Catalog, StarNames, load_embedded_catalog};
use crate::model::Sky;
use crate::scene::draw_sky_scene;

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
    let mut sky = crate::sky::create_sky_from_catalog(&Catalog::new(catalog.stars, names, vec![]));
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

fn project(sky: &Sky) -> crate::model::ProjectionData {
    project_sky(sky, &View::default(), Viewport { height: 40, width: 60 })
}

fn check_pixels(cache: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions) {
    let expected = draw_pixel_sky(sky, options, &mut StepTimes::default()).unwrap();
    let actual = crate::scene::draw_pixels(cache, sky, options, J2000, &mut StepTimes::default())
        .unwrap();
    assert_eq!(actual, expected);
    let hits = cache.pixels.stats.hits;
    assert_eq!(
        crate::scene::draw_pixels(cache, sky, options, J2000, &mut StepTimes::default())
            .unwrap(),
        expected
    );
    assert_eq!(cache.pixels.stats.hits, hits + 1);
}

fn check_characters(cache: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions, size: (usize, usize)) {
    let mut expected = Canvas::new(size.0, size.1);
    draw_sky_scene(&mut expected, options, sky);
    let mut actual = Canvas::new(size.0, size.1);
    crate::scene::draw_characters(cache, &mut actual, sky, options, J2000);
    assert_eq!(actual, expected);
    let hits = cache.characters.stats.hits;
    crate::scene::draw_characters(cache, &mut actual, sky, options, J2000);
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
        let projected_data = project(&sky);
        let projected = projected_data.view(&sky);
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
    let projected_data = project(&sky);
    let projected = projected_data.view(&sky);
    let refreshes = cache.pixels.stats.refreshes;
    check_pixels(&mut cache, &projected, &options());
    assert_eq!(cache.pixels.stats.refreshes, refreshes);
    assert_eq!(std::mem::size_of::<crate::model::PixelStarKey>(), 24);
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
            3 => projected.order.reverse(),
            4 => { projected.order.remove(0); },
            5 => projected.stars[projected.order[0]].1 = (5, 6),
            6 => projected.order.clear(),
            _ => {}
        }
        check_pixels(&mut cache, &projected.view(&sky), &options);
        check_characters(&mut cache, &projected.view(&sky), &options, (40, 60));
    }
}

#[test]
fn character_keys_capture_resolved_names_designations_options_and_canvas_size() {
    let mut cache = SceneCache::default();
    for name in ["First", "Other"] {
        let sky = fixture(name, *b"K0", None);
        let refreshes = cache.characters.stats.refreshes;
        check_characters(&mut cache, &project(&sky).view(&sky), &options(), (40, 60));
        assert_eq!(cache.characters.stats.refreshes, refreshes + 1);
    }
    for (index, name) in ["First", "Other", "α wide 星", "Last"].into_iter().enumerate() {
        // Name IDs have the same offset/length for First/Other; strings themselves must invalidate the key.
        let sky = fixture(name, *b"K0", None);
        let projected_data = project(&sky);
        let projected = projected_data.view(&sky);
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
        let mut sky = crate::sky::create_sky_from_catalog(&Catalog::new(source.stars, StarNames::default(), vec![]));
        sky.stars[0].position = Horizontal {
            azimuth: 0.0,
            altitude: 1.2,
        }
        .to_unit_vector();
        let mut projected = project(&sky);
        projected.planets.iter_mut().for_each(|planet| planet.cell = None);
        projected.moon.cell = None;
        let refreshes = cache.characters.stats.refreshes;
        check_characters(&mut cache, &projected.view(&sky), &options(), (40, 60));
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
        check_pixels(&mut cache, &projected.view(&sky), &options);
        check_characters(&mut cache, &projected.view(&sky), &options, (40, 60));
    }
    cache.configure(&CacheConfig::disabled());
    let expected = draw_pixel_sky(&projected.view(&sky), &options, &mut StepTimes::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            crate::scene::draw_pixels(&mut cache, &projected.view(&sky), &options, J2000, &mut StepTimes::default())
                .unwrap(),
            expected
        );
    }
    assert_eq!(cache.pixels.stats.bypasses, 2);
}

#[test]
fn prepared_display_constants_match_reference_and_fall_back_for_another_catalog() {
    let original = fixture("Original", *b"B0", None);
    let other = fixture("Other", *b"  ", Some(1.4));
    let mut cache = SceneCache::default();
    crate::scene::prepare_scene_catalog(&mut cache, original.catalog.clone(), &mut StepTimes::with_trace(true));
    for sky in [&original, &other] {
        let mut projected = project(sky);
        for phase in 0..3 {
            if phase == 1 {
                projected.order.reverse();
            }
            if phase == 2 {
                projected.order.remove(0);
            }
            check_pixels(&mut cache, &projected.view(sky), &options());
            let expected: Vec<_> = projected.view(sky)
                .stars
                .iter()
                .enumerate()
                .filter_map(|(i, star)| star.star.name().is_some().then_some(i))
                .collect();
            assert_eq!(cache.named_candidates().unwrap(), expected);
            check_characters(&mut cache, &projected.view(sky), &options(), (40, 60));
        }
    }
    cache.configure(&CacheConfig::disabled());
    assert!(
        cache.prepared().is_some(),
        "immutable preparation survives runtime cache bypass"
    );
    assert_eq!(
        crate::scene::draw_pixels(&mut cache, &project(&original).view(&original), &options(), J2000, &mut StepTimes::default())
            .unwrap(),
        draw_pixel_sky(&project(&original).view(&original), &options(), &mut StepTimes::default()).unwrap()
    );
}

#[test]
fn pixel_candidate_reuses_hit_capacity_then_moves_on_refresh() {
    let sky = fixture("Alpha", *b"G0", None);
    let projected = project(&sky);
    let view = projected.view(&sky);
    let mut cache = SceneCache::default();
    let mut times = StepTimes::default();
    let expected = draw_pixels(&mut cache, &view, &options(), J2000, &mut times).unwrap();
    assert!(cache.pixel_candidate.is_none()); // the successful refresh moved the entire key into the cache

    draw_pixels(&mut cache, &view, &options(), J2000, &mut times).unwrap();
    let candidate = cache.pixel_candidate.as_ref().unwrap();
    let StarKeys::Pixels(stars) = &candidate.stars else { panic!("pixel key expected") };
    assert!(stars.is_empty());
    assert!(stars.capacity() >= view.stars.len());
    let allocation = (stars.as_ptr(), stars.capacity());
    let planet_allocation = (candidate.planets.as_ptr(), candidate.planets.capacity());
    for _ in 0..3 {
        assert_eq!(draw_pixels(&mut cache, &view, &options(), J2000, &mut times).unwrap(), expected);
        let candidate = cache.pixel_candidate.as_ref().unwrap();
        let StarKeys::Pixels(stars) = &candidate.stars else { panic!("pixel key expected") };
        assert_eq!((stars.as_ptr(), stars.capacity()), allocation);
        assert_eq!((candidate.planets.as_ptr(), candidate.planets.capacity()), planet_allocation);
        assert!(stars.is_empty() && candidate.planets.is_empty());
    }

    let generation = cache.pixels.generation;
    let refreshes = cache.pixels.stats.refreshes;
    cache.invalidate();
    assert_eq!(draw_pixels(&mut cache, &view, &options(), J2000 + 1.0, &mut times).unwrap(), expected);
    assert!(cache.pixel_candidate.is_none());
    assert_eq!(cache.pixels.stats.refreshes, refreshes + 1);
    assert_eq!(cache.pixels.generation, generation); // equal raster results keep the existing generation
    assert_eq!(cache.pixels.calculated_at, Some(J2000 + 1.0));
}

#[test]
fn character_candidate_clears_labels_without_discarding_flat_capacity() {
    let sky = fixture("Alpha", *b"G0", None);
    let projected = project(&sky);
    let view = projected.view(&sky);
    let mut cache = SceneCache::default();
    let mut canvas = Canvas::new(40, 60);
    draw_characters(&mut cache, &mut canvas, &view, &options(), J2000);
    let expected = canvas.clone();
    assert!(cache.character_candidate.is_none());
    draw_characters(&mut cache, &mut canvas, &view, &options(), J2000);
    let candidate = cache.character_candidate.as_ref().unwrap();
    let StarKeys::Characters { glyphs, labels } = &candidate.stars else { panic!("character key expected") };
    assert!(glyphs.is_empty() && labels.is_empty());
    assert!(labels.capacity() > 0); // resolved label Strings were dropped; the flat list remains reusable
    let allocations = (glyphs.as_ptr(), glyphs.capacity(), labels.as_ptr(), labels.capacity());
    for _ in 0..3 {
        draw_characters(&mut cache, &mut canvas, &view, &options(), J2000);
        let candidate = cache.character_candidate.as_ref().unwrap();
        let StarKeys::Characters { glyphs, labels } = &candidate.stars else { panic!("character key expected") };
        assert_eq!((glyphs.as_ptr(), glyphs.capacity(), labels.as_ptr(), labels.capacity()), allocations);
        assert!(glyphs.is_empty() && labels.is_empty());
        assert_eq!(canvas, expected);
    }

    let generation = cache.characters.generation;
    cache.invalidate();
    draw_characters(&mut cache, &mut canvas, &view, &options(), J2000 + 1.0);
    assert!(cache.character_candidate.is_none());
    assert_eq!(cache.characters.generation, generation);
    assert_eq!(canvas, expected);
}

#[test]
fn failed_pixel_refresh_keeps_committed_value_and_candidate_for_retry() {
    let sky = fixture("Alpha", *b"G0", None);
    let mut projected = project(&sky);
    let mut cache = SceneCache::default();
    let mut times = StepTimes::default();
    let expected = draw_pixels(&mut cache, &projected.view(&sky), &options(), J2000, &mut times).unwrap();
    let generation = cache.pixels.generation;
    let refreshes = cache.pixels.stats.refreshes;
    let viewport = projected.viewport;
    projected.viewport.width = 0; // tiny-skia rejects a zero-width image without allocating a huge buffer
    assert!(draw_pixels(&mut cache, &projected.view(&sky), &options(), J2000 + 1.0, &mut times).is_none());
    assert!(cache.pixel_candidate.is_some());
    assert!(cache.pixels.has_been_invalidated);
    assert_eq!(cache.pixels.generation, generation);
    assert_eq!(cache.pixels.stats.refreshes, refreshes);
    assert_eq!(cache.pixels.calculated_at, Some(J2000));

    projected.viewport = viewport;
    assert_eq!(draw_pixels(&mut cache, &projected.view(&sky), &options(), J2000 + 2.0, &mut times).unwrap(), expected);
    assert!(cache.pixel_candidate.is_none());
    assert_eq!(cache.pixels.generation, generation); // retry compared against the old committed image
    assert_eq!(cache.pixels.stats.refreshes, refreshes + 1);
    assert!(!cache.pixels.has_been_invalidated);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn character_memory_events_preserve_copy_paths_and_skip_drawing_on_hits() {
    use crate::timing::{BufferId, MemoryEvent, Operation};
    let sky = fixture("First", *b"B0", None);
    let data = project(&sky);
    let projected = data.view(&sky);
    let mut cache = SceneCache::default();
    let mut canvas = Canvas::new(40, 60);
    let mut first = StepTimes::with_trace(true);
    first.enable_memory_events(true);
    draw_characters_with_times(&mut cache, &mut canvas, &projected, &options(), J2000, &mut first);
    let expected = canvas.clone();
    assert!(first.trace().unwrap().steps.iter().any(|step| step.name == "Character cache copy" && step.memory_events.iter().any(|e| matches!(e.event, MemoryEvent::Operation { buffer: BufferId::CharacterScene, operation: Operation::Copy, .. }))));
    canvas.clear();
    let mut hit = StepTimes::with_trace(true);
    hit.enable_memory_events(true);
    draw_characters_with_times(&mut cache, &mut canvas, &projected, &options(), J2000, &mut hit);
    assert_eq!(canvas, expected);
    assert!(!hit.trace().unwrap().steps.iter().any(|step| step.name == "Canvas initialization"));
    assert!(hit.trace().unwrap().steps.iter().any(|step| step.name == "Raster output copy" && step.memory_events.iter().any(|e| matches!(e.event, MemoryEvent::Operation { buffer: BufferId::CharacterFrame, operation: Operation::Copy, .. }))));
}
