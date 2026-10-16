//! Real frame sub-steps reach both the timing panel and machine-readable capture without changing pixels.
use astroterm::formulas::{J2000, Observer};
use astroterm::model::{RenderOptions, Data, View};
use astroterm::observation::observe;
use astroterm::points::{draw_points, draw_panel_and_notices};
use astroterm::present::{Presented, Terminal, present};
use astroterm::projection::{Frame, project_stars_to_screen};
use astroterm::regions::refresh_regions;
use astroterm::selection::select_regions;
use astroterm::solar_system::simulate_solar_system;
use astroterm::time::StepTimes;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;

fn render(data: &mut Data, frame: &mut Frame, terminal: &mut Terminal<Vec<u8>>, times: &mut StepTimes) -> Presented {
    let time = astroterm::formulas::utc_jd_to_time_ms(J2000).unwrap();
    let view = View::default();
    let settings = RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 6.0, dynamic_names: true };
    times.begin_frame();
    times.measure("Solar system");
    let solar = simulate_solar_system(&data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons, time, times);
    times.measure("Observation");
    let observation = observe(&Observer::default(), true, &view, time, astroterm::formulas::evaluate_earth_state(time), times);
    times.measure("Selection");
    let selected = select_regions(&data.star_regions, &observation.patch, observation.time_ms, times);
    times.measure("Regions");
    refresh_regions(
        &mut data.star_regions, &data.constellation_stars,
        &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
        &mut data.star_opacities, &mut data.tile_bytes,
        &selected.regions, time, view.fov_degrees, settings.magnitude_threshold,
        times,
    );
    times.measure("Projection");
    let mut projection = astroterm::projection::ProjectionExecutor::default();
    let projected_stars = project_stars_to_screen(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &view, &mut projection, times);
    frame.insert_projected_stars(projected_stars);
    times.measure("Points");
    let drawn = draw_points(
        frame, &solar,
        &data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons,
        &data.constellation_stars, &data.constellation_simulation, &data.constellation_figures,
        &data.star_regions, &data.names, &data.star_opacities, &data.clipping,
        &selected.regions, None,
        &observation, &view, &settings, times,
    );
    times.measure("Text overlay");
    draw_panel_and_notices(frame, &[], &drawn.notices);
    times.measure("Present");
    let presented = present(frame, terminal, times);
    times.measure("END");
    presented.unwrap()
}

#[test]
fn detail_is_hierarchical_bounded_and_preserves_the_rendered_frame() {
    let mut report = astroterm::progress::Report::quiet();
    let catalog = astroterm::catalog::load_embedded_catalog(&mut report).unwrap();
    let prepared = astroterm::sky::prepare_owned_catalog(catalog, &mut report).unwrap();
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut report).unwrap();
    let mut frame = Frame::square(81);
    let mut data = tables.clone();
    let mut terminal = Terminal::new(Vec::new(), ProtocolType::Sixel, false, Rect::new(0, 0, 8, 4), false, false);
    let mut plain = StepTimes::default();
    assert!(matches!(render(&mut data, &mut frame, &mut terminal, &mut plain), Presented::Sent(_)));
    assert_eq!(plain.steps().len(), 8);
    let expected_stars = frame.stars().to_vec();
    let expected_overlay = frame.overlay().to_vec();

    let mut detailed = StepTimes::with_trace(true);
    data = tables;                                                      // exercise drawing and merging again with identical inputs
    assert_eq!(render(&mut data, &mut frame, &mut terminal, &mut detailed), Presented::Unchanged);
    assert_eq!(frame.stars(), expected_stars);
    assert_eq!(frame.overlay(), expected_overlay);
    for name in ["Planet light time", "Observer and orientation", "Brightness prefixes", "Group image rendering", "Region image merging", "Text rasterization", "Frame comparison"] {
        assert_eq!(detailed.steps().iter().filter(|step| step.name == name && step.depth == 1).count(), 1, "{name}");
    }
    for name in ["Region mapping", "Tile filter preparation", "Tile rasterization"] {
        assert_eq!(detailed.trace().unwrap().steps.iter().filter(|step| step.name == name).count(), 1, "{name}");
    }
    assert!(detailed.trace().unwrap().steps.len() < 70);                                  // no row per region or texel
    let capture = serde_json::to_value(detailed.capture_steps().unwrap()).unwrap();
    let child = capture.as_array().unwrap().iter().find(|row| row["name"] == "Brightness prefixes").unwrap();
    assert_eq!(capture[child["parent"].as_u64().unwrap() as usize]["name"], "Regions");
    assert!(detailed.steps().iter().any(|step| step.name == "Text overlay" && step.depth == 0));
    let fields = astroterm::metadata::format_step_time_fields(detailed.steps());
    assert!(fields.iter().any(|field| field.label.contains("Brightness prefixes")));

    detailed.clear_capture_segment().unwrap();
    render(&mut data, &mut frame, &mut terminal, &mut detailed);
    assert!(!detailed.trace().unwrap().steps.iter().any(|step| step.name == "Region image merging" || step.name == "Image encoding")); // cache hit work stays absent, not repeated from the previous trace
}

#[test]
fn failed_presentation_can_finish_its_checkpoint_before_propagating_the_error() {
    struct FailedOutput;
    impl std::io::Write for FailedOutput {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> { Err(std::io::Error::other("test write failure")) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let frame = Frame::square(9);
    let mut terminal = Terminal::new(FailedOutput, ProtocolType::Sixel, false, Rect::new(0, 0, 2, 2), false, false);
    let mut times = StepTimes::with_trace(true);
    times.begin_frame();
    times.measure("Present");
    let result = present(&frame, &mut terminal, &mut times);
    times.measure("END");

    assert_eq!(result.unwrap_err().to_string(), "test write failure");
    assert!(times.capture_steps().is_ok());
    assert!(times.steps().iter().any(|step| step.name == "Image upload"));
}
