//! Fixed-seed catalogs and fixed observer/date/view. Setup and sorting are outside timed loops.

use std::hint::black_box;
use std::time::Duration;

use astroterm::astro::{J2000, Observer};
use astroterm::canvas::Canvas;
use astroterm::catalog::{Catalog, CatalogStar, load_embedded_catalog};
use astroterm::projection::{View, ViewCenter, Viewport, project_sky};
use astroterm::scene::{RenderOptions, draw_sky_scene};
use astroterm::sky::{
    FrameTime, SimulationState, Sky, observe_sky, prepare_observation, update_simulation, update_sky_positions,
};
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
        let ra_motion = (uniform() - 0.5) * 1e-6;
        let dec_motion = (uniform() - 0.5) * 1e-6;
        catalog.stars.push(CatalogStar {
            id: astroterm::catalog::StarId(catalog.stars.len() as u64 + 1),
            space_motion: None,
            hr: None,
            name: None,
            designation: None,
            right_ascension: ra,
            declination: dec,
            ra_motion,
            ra_motion_cos_dec: ra_motion * dec.cos(),
            dec_motion,
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
        let mut sky = Sky::new(std::sync::Arc::new(astroterm::sky::SkyCatalog::from_owned_catalog(
            build_catalog(count),
        )));
        let mut timing = StepTimes::default();
        let mut group = criterion.benchmark_group(name);
        group
            .sample_size(10)
            .warm_up_time(Duration::from_secs(1))
            .measurement_time(Duration::from_secs(2))
            .sampling_mode(SamplingMode::Flat);
        for threshold in [5.0, 12.0, f64::INFINITY] {
            for refracted in [false, true] {
                let correction = if refracted { "refracted" } else { "geometric" };
                let mut simulation = SimulationState::default();
                let mut frame = 0_u64;
                group.bench_function(format!("update_{correction}_t{threshold}"), |bencher| {
                    bencher.iter(|| {
                        let time = FrameTime::from_utc(date + frame as f64 / (24.0 * 86400.0));
                        frame += 1;
                        update_simulation(&mut simulation, time, &[], &mut timing).unwrap();
                        let observer_state = prepare_observation(&mut simulation, time, observer).unwrap();
                        observe_sky(
                            &simulation,
                            &observer_state,
                            threshold,
                            refracted,
                            astroterm::sky::SkyRegion::All,
                            black_box(&mut sky),
                            &mut timing,
                        )
                        .unwrap();
                        black_box(&sky);
                    });
                });
            }
        }
        update_sky_positions(&mut sky, date, &observer, f64::INFINITY, &mut timing);
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
                    update_sky_positions(&mut sky, date, &observer, threshold, &mut timing);
                    group.bench_function(
                        format!("project_draw_{view_name}_t{threshold}_constellations_{constellations}"),
                        |bencher| {
                            bencher.iter(|| {
                                let projected = project_sky(
                                    black_box(&sky),
                                    &view,
                                    Viewport {
                                        height: canvas.height(),
                                        width: canvas.width(),
                                    },
                                );
                                draw_sky_scene(&mut canvas, &options, &projected);
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

fn benchmark_model_families(criterion: &mut Criterion) {
    use astroterm::astro::models::{
        BodyId, moons::evaluate_moon, orientation::compute_slow_orientation, planets::evaluate_planets,
    };
    let mut group = criterion.benchmark_group("models");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2));
    group.bench_function("planet_refresh", |b| {
        b.iter(|| black_box(evaluate_planets(black_box(J2000))))
    });
    group.bench_function("lunar_refresh", |b| {
        b.iter(|| black_box(evaluate_moon(black_box(J2000))))
    });
    group.bench_function("orientation_refresh", |b| {
        b.iter(|| black_box(compute_slow_orientation(black_box(J2000))))
    });
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(J2000);
    update_simulation(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
    group.bench_function("cached_bodies", |b| {
        b.iter(|| {
            for body in BodyId::PLANETS.into_iter().chain([BodyId::Moon]) {
                black_box(
                    simulation
                        .evaluate_body(body, black_box(time.tt + 1.0 / 86400.0))
                        .unwrap(),
                );
            }
        })
    });
    group.finish();
}

criterion_group!(benches, benchmark_frames, benchmark_model_families);
criterion_main!(benches);
