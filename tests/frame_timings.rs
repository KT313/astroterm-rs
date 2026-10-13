//! Real frame sub-steps reach both the timing panel and machine-readable capture without changing pixels.
use astroterm::astro::{FrameTime, J2000, Observer};
use astroterm::model::{RenderOptions, Data, View};
use astroterm::observation::observe;
use astroterm::points::{PointInputs, draw_points};
use astroterm::present::{Presented, Terminal, present};
use astroterm::projection::{Frame, project_stars_to_screen};
use astroterm::regions::refresh_regions;
use astroterm::selection::select_regions;
use astroterm::solar_system::simulate_solar_system;
use astroterm::timing::StepTimes;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;

fn render(data: &mut Data, frame: &mut Frame, terminal: &mut Terminal<Vec<u8>>, times: &mut StepTimes) -> Presented {
    let time = FrameTime::from_utc(J2000);
    let view = View::default();
    let settings = RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 6.0, dynamic_names: true };
    times.begin_frame();
    let solar = times.measure_steps("Solar system", |times| simulate_solar_system(data, time, times));
    let observation = times.measure_steps("Observation", |times| observe(&Observer::default(), true, &view, time, times));
    let selected = times.measure_steps("Selection", |times| select_regions(&data.star_regions, &observation, times));
    times.measure_steps("Regions", |times| refresh_regions(data, &selected, time, &view, &settings, times));
    times.measure_steps("Projection", |times| project_stars_to_screen(frame, data, &selected, &observation, &view, times));
    let inputs = PointInputs { solar_system: &solar, data, selected: &selected, panel: &[], notice: None };
    times.measure_steps("Points", |times| draw_points(frame, &inputs, &observation, &view, &settings, times));
    times.measure_steps("Present", |times| present(frame, terminal, times)).unwrap()
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
    assert_eq!(plain.steps().len(), 7);
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
    if astroterm::constants::FORWARD_TILE_PROJECTION {
        for name in ["Texel projection", "Triangle rasterization", "Tile composition"] {
            assert_eq!(detailed.trace().unwrap().steps.iter().filter(|step| step.name == name).count(), 1, "{name}");
        }
    }
    assert!(detailed.trace().unwrap().steps.len() < 70);                                  // no row per region or texel
    let capture = serde_json::to_value(detailed.capture_steps().unwrap()).unwrap();
    let child = capture.as_array().unwrap().iter().find(|row| row["name"] == "Brightness prefixes").unwrap();
    assert_eq!(capture[child["parent"].as_u64().unwrap() as usize]["name"], "Regions");
    let fields = astroterm::metadata::format_step_time_fields(detailed.steps());
    assert!(fields.iter().any(|field| field.label.contains("Brightness prefixes")));

    detailed.clear_capture_segment().unwrap();
    render(&mut data, &mut frame, &mut terminal, &mut detailed);
    assert!(!detailed.trace().unwrap().steps.iter().any(|step| step.name == "Region image merging" || step.name == "Image encoding")); // cache hit work stays absent, not repeated from the previous trace
}
