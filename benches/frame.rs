//! Fixed-seed catalogs and fixed observer/date/view. Setup and sorting are outside timed loops.

use std::hint::black_box;
use std::time::Duration;

use astroterm::astro::{J2000, Observer};
use astroterm::canvas::Canvas;
use astroterm::catalog::{Catalog, CatalogStar, load_embedded_catalog};
use astroterm::projection::{View, ViewCenter};
use astroterm::scene::{RenderOptions, draw_sky_scene};
use astroterm::sky::{Sky, refract_sky_positions, update_sky_positions};
use astroterm::timing::StepTimes;
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};

/// Deterministic catalog with real bright stars/figures and a synthetic faint tail similar to AT-HYG's counts.
fn build_catalog(count: usize) -> Catalog {
    let mut catalog = load_embedded_catalog().unwrap();
    if count <= catalog.stars.len() {
        return catalog;
    }
    let mut seed = 0x415354524f_u64;
    let mut uniform = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    catalog.stars.reserve(count - catalog.stars.len());
    while catalog.stars.len() < count {
        let ra = uniform() * std::f64::consts::TAU;
        let dec = (2.0 * uniform() - 1.0).asin();
        let quantile = uniform();
        let magnitude = if quantile < 0.05 {
            5.0 + 4.0 * uniform()
        } else if quantile < 0.34 {
            9.0 + 2.0 * uniform()
        } else {
            11.0 + 2.0 * uniform()
        };
        catalog.stars.push(CatalogStar {
            id: astroterm::catalog::StarId(catalog.stars.len() as u64 + 1),
            space_motion: None,
            hr: None,
            name: None,
            designation: None,
            right_ascension: ra,
            declination: dec,
            ra_motion: (uniform() - 0.5) * 1e-6,
            dec_motion: (uniform() - 0.5) * 1e-6,
            magnitude: magnitude as f32,
            spectral_type: *b"G2",
            color_index: None,
            has_data: true,
        });
    }
    catalog
}

fn benchmark_frames(criterion: &mut Criterion) {
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };
    let date = J2000 + 25.0 * 365.25;
    for (name, count) in [
        ("embedded", 9110),
        ("synthetic_100k", 100_000),
        ("synthetic_2500k", 2_500_000),
    ] {
        let mut sky = Sky::from_catalog(&build_catalog(count));
        let mut timing = StepTimes::default();
        let mut group = criterion.benchmark_group(name);
        group
            .sample_size(10)
            .warm_up_time(Duration::from_secs(1))
            .measurement_time(Duration::from_secs(2))
            .sampling_mode(SamplingMode::Flat);
        for threshold in [5.0, 12.0, f32::INFINITY] {
            for refracted in [false, true] {
                let correction = if refracted { "refracted" } else { "geometric" };
                group.bench_function(format!("update_{correction}_t{threshold}"), |bencher| {
                    bencher.iter(|| {
                        update_sky_positions(black_box(&mut sky), black_box(date), &observer, threshold, &mut timing);
                        if refracted {
                            refract_sky_positions(&mut sky);
                        }
                        black_box(&sky);
                    });
                });
            }
        }
        update_sky_positions(&mut sky, date, &observer, f32::INFINITY, &mut timing);
        let mut canvas = Canvas::new(41, 81);
        for (view_name, fov) in [("wide", 180.0), ("narrow", 10.0)] {
            let view = View {
                center: ViewCenter::Facing {
                    azimuth: 225_f64.to_radians(),
                    tilt: 30_f64.to_radians(),
                },
                fov_degrees: fov,
                ..View::default()
            };
            for constellations in [false, true] {
                for threshold in [5.0, 12.0] {
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
                    group.bench_function(
                        format!("draw_{view_name}_t{threshold}_constellations_{constellations}"),
                        |bencher| {
                            bencher.iter(|| {
                                draw_sky_scene(&mut canvas, &view, &options, black_box(&sky));
                                black_box(&canvas);
                            });
                        },
                    );
                }
            }
        }
        group.finish();
    }
}

criterion_group!(benches, benchmark_frames);
criterion_main!(benches);
