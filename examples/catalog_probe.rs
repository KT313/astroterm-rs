//! Measure dataset loading separately from sky construction and frame work (release builds).

use astroterm::state::{SimulationState};
use std::{hint::black_box, path::Path, time::Instant};

use astroterm::astro::Observer;
use astroterm::canvas::Canvas;
use astroterm::catalog::load_athyg_catalog;
use astroterm::model::Sky;
use astroterm::model::projection::{ProjectionViewport as Viewport, View};
use astroterm::model::rendering::RenderOptions;
use astroterm::model::simulation::FrameTime;
use astroterm::projection::project_sky;
use astroterm::scene::draw_sky_scene;
use astroterm::sky::{observe_sky, prepare_observation, update_simulation};
use astroterm::timing::StepTimes;

fn main() {
    // measure loading separately from sky construction
    let path = std::env::args().nth(1).expect("dataset path");
    let start = Instant::now();
    let catalog = load_athyg_catalog(Path::new(&path)).unwrap();
    println!(
        "load_ms={:.3} stars={}",
        start.elapsed().as_secs_f64() * 1000.0,
        catalog.stars.len()
    );

    // prepare the fixed rendering workload
    let start = Instant::now();
    let mut sky = Sky::new(std::sync::Arc::new(astroterm::sky::prepare_owned_catalog(catalog)));
    println!(
        "prepare_ms={:.3} singular={} always_checked={} endpoints={} precise={}",
        start.elapsed().as_secs_f64() * 1000.0,
        sky.catalog.singular_count,
        sky.catalog.always_checked.len(),
        sky.catalog.endpoint_indices.len(),
        sky.catalog.stars.precise_count()
    );
    if std::env::args().any(|arg| arg == "--matrix") {
        measure_matrix(&mut sky);
        return;
    }
    let mut canvas = Canvas::new(41, 81);
    let view = View::default();
    let options = RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    };
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };

    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(2460736.9583333335);
    let mut timing = StepTimes::default();
    let mut update = |sky: &mut Sky| {
        update_simulation(&mut simulation, time, &[], &mut timing).unwrap();
        let site = prepare_observation(&mut simulation, time, observer).unwrap();
        observe_sky(&simulation, &site, 5.0, false, astroterm::projection::select_view_region(&view), sky, &mut timing).unwrap();
    };

    // time position updates, then complete headless frames
    let start = Instant::now();
    for _ in 0..20 {
        update(black_box(&mut sky));
    }
    println!("update_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0 / 20.0);

    let start = Instant::now();
    for _ in 0..100 {
        update(black_box(&mut sky));
        let projected_data = project_sky(
            &sky,
            &view,
            Viewport {
                height: canvas.height(),
                width: canvas.width(),
            },
        );
        let projected = projected_data.view(&sky);
        draw_sky_scene(black_box(&mut canvas), &options, &projected);
    }
    println!(
        "update_draw_ms={:.3} drawable={} evaluated={} brightness_candidates={}",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0,
        sky.count_bright_stars(5.0),
        sky.stars.len(),
        sky.catalog.count_bright_stars(5.0)
    );
}

/// Fixed Tokyo/date, 41x81 character canvas; simulation is paused after one cold model refresh.
fn measure_matrix(sky: &mut Sky) {
    let time = FrameTime::from_utc(2460736.9583333335);
    let site = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };
    let mut simulation = SimulationState::default();
    let mut canvas = Canvas::new(41, 81);
    for (threshold, fov) in [(5.0, 180.0), (12.0, 10.0), (12.0, 180.0)] {
        let view = View {
            center: astroterm::model::projection::ViewCenter::Facing {
                azimuth: 225_f64.to_radians(),
                tilt: 30_f64.to_radians(),
            },
            fov_degrees: fov,
            ..View::default()
        };
        for refraction in [false, true] {
            for constellations in [false, true] {
                let options = RenderOptions {
                    unicode: true,
                    braille: true,
                    color: true,
                    constellations,
                    grid: false,
                    magnitude_threshold: threshold,
                    label_threshold: 0.25,
                    dynamic_names: true,
                };
                let mut timing = StepTimes::default();
                let mut elapsed = 0.0;
                for frame in 0..100 {
                    let start = Instant::now();
                    timing
                        .measure_steps("Simulation", |steps| {
                            update_simulation(&mut simulation, time, &[], steps)
                        })
                        .unwrap();
                    timing.measure_steps("Observation", |steps| {
                        let observer = prepare_observation(&mut simulation, time, site).unwrap();
                        observe_sky(
                            &simulation,
                            &observer,
                            threshold,
                            refraction,
                            astroterm::projection::select_view_region(&view),
                            sky,
                            steps,
                        )
                        .unwrap();
                    });
                    let projected_data = timing.measure("Projection", || {
                        project_sky(sky, &view, Viewport { height: 41, width: 81 })
                    });
                    let projected = projected_data.view(sky);
                    timing.measure("Draw", || draw_sky_scene(&mut canvas, &options, &projected));
                    if frame >= 20 {
                        elapsed += start.elapsed().as_secs_f64();
                    }
                    black_box(&canvas);
                }
                let steps: std::collections::BTreeMap<_, _> = timing
                    .steps()
                    .iter()
                    .map(|s| (s.name, s.average_seconds * 1000.0))
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({"threshold":threshold,"fov":fov,"refraction":refraction,
                "constellations":constellations,"mean_frame_ms":elapsed*1000.0/80.0,"steps_ms_ema":steps,
                "cells":sky.selection.cells,"candidates":sky.selection.candidates,"evaluated":sky.stars.len()})
                );
            }
        }
    }
}
