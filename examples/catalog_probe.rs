//! Measure dataset loading separately from sky construction and frame work (release builds).

use std::{hint::black_box, path::Path, time::Instant};

use astroterm::{
    astro::Observer,
    canvas::Canvas,
    catalog::load_athyg_catalog,
    projection::{View, Viewport, project_sky},
    scene::{RenderOptions, draw_sky_scene},
    sky::{FrameTime, SimulationState, Sky, SkyCatalog, observe_sky, prepare_observer, update_simulation},
    timing::StepTimes,
};

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
    let mut sky = Sky::new(std::sync::Arc::new(SkyCatalog::from_catalog(&catalog)));
    drop(catalog);
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
        let site = prepare_observer(&simulation, time, observer).unwrap();
        observe_sky(&simulation, &site, 5.0, false, sky, &mut timing).unwrap();
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
        let projected = project_sky(
            &sky,
            &view,
            Viewport {
                height: canvas.height(),
                width: canvas.width(),
            },
        );
        draw_sky_scene(black_box(&mut canvas), &options, &projected);
    }
    println!(
        "update_draw_ms={:.3} candidates={}",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0,
        sky.count_bright_stars(5.0)
    );
}
