//! Reference-image checks and an opt-in real-catalog comparison for the minimum-star fast path.
use crate::state::{SimulationState};
use super::*;
use crate::astro::Observer;
use crate::catalog::{Catalog, load_embedded_catalog};
use crate::model::{ObservedSky, SkyCatalog, ProjectionViewport as Viewport, View, FrameTime};
use crate::projection::project_sky;
use crate::sky::{observe_sky, prepare_observation, update_solar_system};
use std::{sync::Arc, time::Instant};

fn options(threshold: f64) -> RenderOptions {
    RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: true,
        magnitude_threshold: threshold,
        dynamic_names: true,
    }
}
fn observe(catalog: Arc<SkyCatalog>, threshold: f64) -> ObservedSky {
    let mut sky = ObservedSky::new(catalog);
    let time = FrameTime::from_utc(2460735.9583333335);
    let mut simulation = SimulationState::default();
    update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
    let site = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };
    let observer = prepare_observation(&mut simulation, time, site).unwrap();
    observe_sky(
        &simulation,
        &observer,
        threshold,
        false,
        crate::model::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    sky
}

#[test]
fn complete_scenes_match_for_mixed_radii_and_large_canvas_fallback() {
    let source = load_embedded_catalog().unwrap();
    let mut stars = source.stars;
    for (i, star) in stars.iter_mut().enumerate() {
        star.magnitude = [2.0, 7.0, 7.03125, 7.03124, 8.0, 10.0][i % 6];
    }
    let catalog = Catalog::new(stars, source.names, source.constellations);
    let sky = observe(Arc::new(crate::sky::prepare_owned_catalog(catalog).unwrap().catalog), 10.0);
    for (width, height) in [(1, 1), (200, 200), (1102, 1102), (4097, 17), (17, 4097)] {
        let view = View {
            fov_degrees: 225.0,
            ..View::default()
        };
        let projected_data = project_sky(&sky, &view, Viewport { width, height });
        let projected = projected_data.view(&sky);
        let actual = draw_pixel_sky(&projected, &options(10.0), &mut StepTimes::default()).unwrap();
        let expected =
            draw_pixel_sky_with_star_path(&projected, &options(10.0), &mut StepTimes::default(), false).unwrap();
        assert_eq!(actual, expected, "{width}x{height}");
    }
}

#[test]
fn prepared_inputs_preserve_the_previous_star_pixels_and_order() {
    let mut source = load_embedded_catalog().unwrap();
    source.stars.truncate(8);
    let mut sky = crate::sky::create_sky_from_catalog(&source).unwrap();
    for (i, star) in sky.stars.iter_mut().enumerate() {
        star.magnitude = [-1.0, 2.0, 5.0, 7.03125, 10.0, 20.0, 8.0, 4.0][i];
        star.position = crate::astro::Vector3 { x: 0.0, y: 0.0, z: 1.0 };
    }
    let mut data = project_sky(&sky, &View::default(), Viewport { width: 64, height: 48 });
    for (i, (_, cell)) in data.stars.iter_mut().enumerate() {
        *cell = [(0, 0), (47, 63), (24, 32), (24, 32)][i % 4]; // include overlaps and partially clipped stars
    }
    for threshold in [5.0, 10.0, 20.0] {
        let options = options(threshold);
        let projected = data.view(&sky);
        let mut cache = crate::state::SceneCache::default();
        crate::scene::caching::prepare_pixel_candidate(&mut cache, &projected, &options, 0.0, &mut StepTimes::default());
        let crate::model::StarKeys::Pixels(inputs) = &cache.pixel_candidate.as_ref().unwrap().stars else { unreachable!() };
        let mut actual = initialize_pixel_canvas(projected.viewport).unwrap();
        draw_pixel_stars(&mut actual, inputs.iter().copied(), true);

        let mut expected = initialize_pixel_canvas(projected.viewport).unwrap();
        for entry in projected.stars.iter() {
            if entry.star.magnitude > threshold { continue; }
            let Some((y, x)) = entry.cell else { continue; };
            let radius = (2.8 - 0.32 * entry.star.magnitude).clamp(0.55, 4.0) as f32;
            let strength = (1.0 - 0.045 * (entry.star.magnitude + 1.46)).clamp(0.16, 1.0);
            let rgb = star_rgb(&entry.star).map(|c| (f64::from(c) * strength).round() as u8);
            draw_disc(&mut expected, x as f32, y as f32, radius, rgb); // independent original circle path
        }
        assert_eq!(actual.data(), expected.data(), "threshold={threshold}");
    }
}

/// Compare actual uncached redraws; scene reuse cannot hide either rasterizer's cost. Source and image paths are
/// explicit environment inputs so normal tests neither load the external catalog nor create image artifacts.
#[test]
#[ignore = "release pixel raster comparison; optionally set ASTROTERM_DATASET and ASTROTERM_RASTER_OUTPUT"]
fn compare_minimum_star_rasterizers() {
    use crate::catalog::datasets::{Dataset, DatasetDirectories};
    let dataset = std::env::var_os("ASTROTERM_DATASET").map(|p| Dataset::Path(p.into()));
    let directories = DatasetDirectories {
        data: None,
        cache: Some(std::env::temp_dir().join("astroterm-processing-probe")),
    };
    let catalog =
        Arc::new(crate::sky::load_sky_catalog(dataset.as_ref(), &directories, &mut std::io::stderr()).unwrap().catalog);
    for threshold in [5.0, 10.0] {
        let sky = observe(catalog.clone(), threshold);
        for fov in [225.0, 115.2, 12.4] {
            let view = View {
                fov_degrees: fov,
                ..View::default()
            };
            let projected_data = project_sky(
                &sky,
                &view,
                Viewport {
                    width: 1102,
                    height: 1102,
                },
            );
            let projected = projected_data.view(&sky);
            let mut durations = [Vec::new(), Vec::new()];
            let mut star_durations = [Vec::new(), Vec::new()];
            for frame in 0..8 {
                let mut images = [None, None];
                for mode in if frame % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let mut times = StepTimes::default();
                    let start = Instant::now();
                    images[mode] =
                        draw_pixel_sky_with_star_path(&projected, &options(threshold), &mut times, mode == 1);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if frame > 0 {
                        durations[mode].push(elapsed);
                        star_durations[mode].push(
                            times
                                .steps()
                                .iter()
                                .find(|s| s.name == "Raster stars")
                                .unwrap()
                                .average_seconds
                                * 1000.0,
                        );
                    }
                }
                assert_eq!(images[0], images[1], "threshold={threshold} fov={fov} frame={frame}");
                if frame == 0
                    && threshold == 10.0
                    && let Some(directory) = std::env::var_os("ASTROTERM_RASTER_OUTPUT")
                {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    for (mode, image) in images.iter().enumerate() {
                        image
                            .as_ref()
                            .unwrap()
                            .save(directory.join(format!("{fov}-{}.png", if mode == 0 { "reference" } else { "fast" })))
                            .unwrap();
                    }
                }
                std::hint::black_box(images);
            }
            let mean = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
            let minimum_stars = projected
                .stars
                .iter()
                .filter(|s| (2.8 - 0.32 * s.star.magnitude).clamp(0.55, 4.0) as f32 == MINIMUM_STAR_RADIUS)
                .count();
            println!(
                "{}",
                serde_json::json!({"threshold":threshold,"fov":fov,"visible_stars":projected.stars.len(),
                "minimum_stars":minimum_stars,"reference_raster_ms":mean(&durations[0]),"fast_raster_ms":mean(&durations[1]),
                "reference_stars_ms":mean(&star_durations[0]),"fast_stars_ms":mean(&star_durations[1]),"different_bytes":0,"measured_frames":7})
            );
        }
    }
}
