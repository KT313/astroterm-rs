//! Phase-4 workload matrix. Set ASTROTERM_BENCH_DATASET to time a local AT-HYG file; otherwise use BSC5.
use astroterm::state::{SimulationState};
use astroterm::astro::Observer;
use astroterm::canvas::Canvas;
use astroterm::catalog::{load_athyg_catalog, load_embedded_catalog};
use astroterm::model::{Sky, ProjectionViewport as Viewport, View, ViewCenter, RenderOptions, FrameTime};
use astroterm::projection::project_sky;
use astroterm::scene::draw_sky_scene;
use astroterm::sky::{observe_sky, prepare_observation, update_solar_system};
use astroterm::timing::StepTimes;
use criterion::{Criterion, SamplingMode, criterion_group, criterion_main};
use std::{hint::black_box, path::Path, sync::Arc, time::Duration};

fn benchmark_spatial(criterion: &mut Criterion) {
    let source = match std::env::var_os("ASTROTERM_BENCH_DATASET") {
        Some(path) => load_athyg_catalog(Path::new(&path)).unwrap(),
        None => load_embedded_catalog().unwrap(),
    };
    let mut sky = Sky::new(Arc::new(astroterm::sky::prepare_owned_catalog(source).unwrap().catalog));
    let mut simulation = SimulationState::default();
    let mut timing = StepTimes::default();
    let time = FrameTime::from_utc(2460736.9583333335);
    update_solar_system(&mut simulation, time, &[], &mut timing).unwrap();
    let observer = prepare_observation(
        &mut simulation,
        time,
        Observer {
            latitude: 35.69_f64.to_radians(),
            longitude: 139.69_f64.to_radians(),
        },
    )
    .unwrap();
    let mut group = criterion.benchmark_group("spatial");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2))
        .sampling_mode(SamplingMode::Flat);
    let mut canvas = Canvas::new(41, 81);
    for (threshold, fov) in [(5.0, 180.0), (12.0, 10.0), (12.0, 180.0)] {
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 225_f64.to_radians(),
                tilt: 30_f64.to_radians(),
            },
            fov_degrees: fov,
            ..View::default()
        };
        for refraction in [false, true] {
            for constellations in [false, true] {
                let name = format!("t{threshold}_fov{fov}_refraction{refraction}_constellations{constellations}");
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
                group.bench_function(format!("observe/{name}"), |b| {
                    b.iter(|| {
                        observe_sky(
                            &simulation,
                            &observer,
                            threshold,
                            refraction,
                            astroterm::projection::select_view_region(&view),
                            black_box(&mut sky),
                            &mut timing,
                        )
                        .unwrap();
                    })
                });
                observe_sky(
                    &simulation,
                    &observer,
                    threshold,
                    refraction,
                    astroterm::projection::select_view_region(&view),
                    &mut sky,
                    &mut timing,
                )
                .unwrap();
                group.bench_function(format!("project_draw/{name}"), |b| {
                    b.iter(|| {
                        let projected_data = project_sky(black_box(&sky), &view, Viewport { height: 41, width: 81 });
                        let projected = projected_data.view(black_box(&sky));
                        draw_sky_scene(&mut canvas, &options, &projected);
                        black_box(&canvas);
                    })
                });
            }
        }
    }
    let mut indices = Vec::new();
    for (name, fov) in [("depth4", 180.0), ("depth6", 10.0)] {
        let region = astroterm::projection::select_view_region(&View {
            fov_degrees: fov,
            ..View::default()
        });
        group.bench_function(format!("query_{name}"), |b| {
            b.iter(|| {
                black_box(
                    astroterm::sky::select_grid(&sky.catalog
                        .grid, &sky.catalog.stars, region, &observer, 12.0, true, &mut indices),
                );
            })
        });
    }
    group.finish();
}
criterion_group!(benches, benchmark_spatial);
criterion_main!(benches);
