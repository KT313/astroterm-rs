//! Measure dataset loading separately from sky construction and frame work (release builds).

use std::{hint::black_box, path::Path, time::Instant};

use astroterm::{
    astro::Observer,
    canvas::Canvas,
    catalog::load_athyg_catalog,
    projection::View,
    scene::{RenderOptions, draw_sky_scene},
    sky::{Sky, update_sky_positions},
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
    let mut sky = Sky::from_catalog(&catalog);
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

    // time position updates, then complete headless frames
    let start = Instant::now();
    for _ in 0..20 {
        update_sky_positions(
            black_box(&mut sky),
            2460736.9583333335,
            &observer,
            5.0,
            &mut StepTimes::default(),
        );
    }
    println!("update_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0 / 20.0);

    let start = Instant::now();
    for _ in 0..100 {
        update_sky_positions(
            black_box(&mut sky),
            2460736.9583333335,
            &observer,
            5.0,
            &mut StepTimes::default(),
        );
        draw_sky_scene(black_box(&mut canvas), &view, &options, &sky);
    }
    println!(
        "update_draw_ms={:.3} candidates={}",
        start.elapsed().as_secs_f64() * 1000.0 / 100.0,
        sky.count_bright_stars(5.0)
    );
}
