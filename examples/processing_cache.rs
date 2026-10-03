//! Controlled headless cache comparison. Optional dataset path; JSON lines exclude terminal encoding/presentation.
use astroterm::{
    astro::Observer,
    cache::CacheConfig,
    catalog::datasets::{Dataset, DatasetDirectories},
    projection::{ProjectionCache, View, Viewport},
    scene::{RenderOptions, cached::SceneCache},
    sky::{FrameTime, ObservationCache, ObservedSky, SimulationState, cache::load_sky_catalog, update_simulation},
    timing::StepTimes,
};
use std::{hint::black_box, sync::Arc, time::Instant};
fn main() {
    let dataset = std::env::args_os().nth(1).map(|p| Dataset::Path(p.into()));
    let directories = DatasetDirectories {
        data: None,
        cache: Some(std::env::temp_dir().join("astroterm-processing-probe")),
    };
    let catalog = Arc::new(load_sky_catalog(dataset.as_ref(), &directories, &mut std::io::stderr()).unwrap());
    for fov in [225.0, 115.2, 12.4] {
        for speed in [0.0, 1.0, 100.0, -100.0, 100000.0] {
            for enabled in [true, false] {
                let config = CacheConfig {
                    enabled,
                    ..CacheConfig::default()
                };
                let mut simulation = SimulationState::default();
                simulation.configure_cache(&config);
                let mut observation = ObservationCache::new(config.clone());
                let mut projection = ProjectionCache::new(config.clone());
                let mut raster = SceneCache::default();
                raster.configure(&config);
                let mut sky = ObservedSky::new(catalog.clone());
                let mut times = StepTimes::default();
                let view = View {
                    fov_degrees: fov,
                    ..View::default()
                };
                let viewport = Viewport {
                    height: 1102,
                    width: 1102,
                };
                let site = Observer {
                    latitude: 35.69_f64.to_radians(),
                    longitude: 139.69_f64.to_radians(),
                };
                let options = RenderOptions {
                    unicode: true,
                    braille: false,
                    color: true,
                    constellations: true,
                    grid: false,
                    magnitude_threshold: 10.0,
                    label_threshold: 0.25,
                    dynamic_names: true,
                };
                let mut totals = [0.0; 4];
                let mut refresh_totals = [0.0; 4];
                let mut refresh_frames = [0; 4];
                for frame in 0..8 {
                    let time = FrameTime::from_utc(2460735.9583333335 + speed * frame as f64 / (12.0 * 86400.0));
                    times.begin_frame();
                    let mut elapsed = [0.0; 4];
                    let previous = [
                        simulation.refresh_counts.planets
                            + simulation.refresh_counts.moon
                            + simulation.refresh_counts.orientation,
                        observation.stats().refreshes,
                        projection.stats().refreshes,
                        raster.stats().refreshes,
                    ];
                    let start = Instant::now();
                    simulation.begin_frame();
                    update_simulation(&mut simulation, time, &[], &mut times).unwrap();
                    elapsed[0] = start.elapsed().as_secs_f64();
                    let simulation_after = simulation.refresh_counts.planets
                        + simulation.refresh_counts.moon
                        + simulation.refresh_counts.orientation;
                    let start = Instant::now();
                    let mut observer = observation.prepare_observer(&simulation, time, site).unwrap();
                    observation
                        .prepare_light_time(&mut simulation, &mut observer, &mut times)
                        .unwrap();
                    observation
                        .observe(
                            &simulation,
                            &observer,
                            10.0,
                            false,
                            view.sky_region(),
                            &mut sky,
                            &mut times,
                        )
                        .unwrap();
                    elapsed[1] = start.elapsed().as_secs_f64();
                    let start = Instant::now();
                    let projected = projection.project(&sky, &view, viewport, time.tt, &mut times);
                    elapsed[2] = start.elapsed().as_secs_f64();
                    let start = Instant::now();
                    black_box(raster.draw_pixels(&projected, &options, time.tt, &mut times).unwrap());
                    elapsed[3] = start.elapsed().as_secs_f64();
                    let current = [
                        simulation_after,
                        observation.stats().refreshes,
                        projection.stats().refreshes,
                        raster.stats().refreshes,
                    ];
                    if frame >= 2 {
                        for i in 0..4 {
                            totals[i] += elapsed[i] * 1000.0;
                            if current[i] > previous[i] {
                                refresh_totals[i] += elapsed[i] * 1000.0;
                                refresh_frames[i] += 1;
                            }
                        }
                    }
                }
                println!(
                    "{}",
                    serde_json::json!({"stars":catalog.stars.len(),"fov":fov,"speed":speed,"cache":enabled,
                    "viewport":[1102,1102],"frames":6,"mean_ms":totals.map(|v|v/6.0),
                    "refresh_frame_mean_ms":(0..4).map(|i|if refresh_frames[i]>0 {Some(refresh_totals[i]/f64::from(refresh_frames[i]))} else {None}).collect::<Vec<_>>(),
                    "stage_order":["simulation","observation","projection","raster"],
                    "observation_hits":observation.stats().hits,"projection_hits":projection.stats().hits,"raster_hits":raster.stats().hits,
                    "model_evaluations":[simulation.refresh_counts.planets,simulation.refresh_counts.moon,simulation.refresh_counts.orientation]})
                );
            }
        }
    }
}
