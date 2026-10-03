//! Single-frame diagnostics must describe production results without changing them.
use astroterm::{
    astro::{J2000, Observer},
    cache::CacheConfig,
    catalog::datasets::{Dataset, DatasetDirectories},
    cli::{Arguments, build_config},
    projection::{ProjectionCache, View, Viewport},
    scene::{RenderOptions, pixels::draw_pixel_sky},
    sky::{FrameTime, ObservationCache, SimulationState, Sky, cache::load_sky_catalog_with_times, update_simulation},
    timing::StepTimes,
};
use clap::Parser;
use std::sync::Arc;

#[test]
fn singleframe_is_independent_of_the_metadata_panel_and_cache_bypass() {
    let args = Arguments::try_parse_from(["astroterm", "--debug-singleframe", "--disable-cache"]).unwrap();
    let config = build_config(args, &[]).unwrap();
    assert!(config.debug_singleframe);
    assert!(!config.terminal.frame_times && !config.terminal.metadata_panel);
    assert!(!config.cache.enabled);
}

#[test]
fn source_and_mapped_loads_report_actual_work_and_ordered_skip_reasons() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("small.csv");
    std::fs::write(&path, "ra,dec,mag\n0,0,4\n1,2,5\n,0,-26\n1,,4\n1,2,\n0,0,-26\n").unwrap();
    let dirs = DatasetDirectories {
        data: None,
        cache: Some(temp.path().join("cache")),
    };
    for warm in [false, true] {
        let mut times = StepTimes::with_trace(true);
        let catalog =
            load_sky_catalog_with_times(Some(&Dataset::Path(path.clone())), &dirs, &mut Vec::new(), &mut times)
                .unwrap();
        assert_eq!(catalog.stars.len(), 2);
        assert_eq!(catalog.stars.is_mapped(), warm);
        let mut report = Vec::new();
        times.trace().unwrap().write_report(&mut report).unwrap();
        let report = String::from_utf8(report).unwrap();
        if warm {
            assert!(report.contains("source CSV not read; original skipped-row counts unavailable"));
            assert!(!report.contains("CSV validation and parsing:"));
        } else {
            assert!(report.contains(
                "input rows=6; removed missing RA/Dec/magnitude=3; then removed Sun (mag < -20)=1; output stars=2"
            ));
            assert!(report.find("CSV validation and parsing:").unwrap() < report.find("Catalog preparation:").unwrap());
        }
    }
}

#[test]
fn tracing_preserves_observation_projection_and_raster_with_cache_or_bypass() {
    let catalog = Arc::new(astroterm::sky::SkyCatalog::from_owned_catalog(
        astroterm::catalog::load_embedded_catalog().unwrap(),
    ));
    let options = RenderOptions {
        unicode: true,
        braille: false,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    };
    for config in [CacheConfig::default(), CacheConfig::disabled()] {
        let mut baseline = None;
        for trace in [false, true] {
            let mut times = StepTimes::with_trace(trace);
            times.begin_frame();
            let mut simulation = SimulationState::default();
            simulation.configure_cache(&config);
            let mut observation = ObservationCache::new(config.clone());
            let mut projection = ProjectionCache::new(config.clone());
            let mut sky = Sky::new(catalog.clone());
            let time = FrameTime::from_utc(J2000);
            let view = View::default();
            update_simulation(&mut simulation, time, &[], &mut times).unwrap();
            let mut observer = observation
                .prepare_observer(&simulation, time, Observer::default())
                .unwrap();
            observation
                .prepare_light_time(&mut simulation, &mut observer, &mut times)
                .unwrap();
            observation
                .observe(
                    &simulation,
                    &observer,
                    5.0,
                    true,
                    view.sky_region(),
                    &mut sky,
                    &mut times,
                )
                .unwrap();
            let projected = projection.project(
                &sky,
                &view,
                Viewport {
                    width: 160,
                    height: 160,
                },
                time.tt,
                &mut times,
            );
            let image = draw_pixel_sky(&projected, &options, &mut times).unwrap();
            let ids = projected
                .stars
                .iter()
                .map(|s| (s.star.id(), s.cell))
                .collect::<Vec<_>>();
            let result = (sky.stars.clone(), ids, image);
            if let Some(baseline) = &baseline {
                assert_eq!(&result, baseline);
            } else {
                baseline = Some(result);
            }
            if let Some(trace) = times.trace() {
                let step = |name| trace.steps.iter().find(|s| s.name == name).unwrap();
                assert!(step("Region filtering").details[0].contains(&format!("input stars={}", catalog.stars.len())));
                assert!(
                    step("Correction selection").details[0]
                        .contains(&format!("output corrected stars={}", sky.stars.len()))
                );
                assert!(
                    step("Raster stars").details[0].contains(&format!("submitted stars={}", projected.stars.len()))
                );
                assert!(step("Refraction").details[0].contains("no membership filtering"));
                let names = trace.steps.iter().map(|s| s.name).collect::<Vec<_>>();
                for (before, after) in [
                    ("Region filtering", "Brightness bounds"),
                    ("Current brightness", "Correction selection"),
                    ("Correction selection", "Aberration"),
                    ("Star projection", "Star draw order"),
                    ("Raster stars", "Raster planets"),
                ] {
                    assert!(
                        names.iter().position(|&n| n == before).unwrap()
                            < names.iter().position(|&n| n == after).unwrap()
                    );
                }
            }
        }
    }
}
