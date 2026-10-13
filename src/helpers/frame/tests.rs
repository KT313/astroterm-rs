use super::*;
use astroterm::astro::J2000;
use astroterm::cli::{Arguments, build_config};
use astroterm::model::{ProjectionKind, ViewCenter};
use astroterm::observation::observe;
use astroterm::present::CompressionSupport;
use astroterm::regions::refresh_regions;
use astroterm::selection::select_regions;
use astroterm::solar_system::simulate_solar_system;
use clap::Parser;
use ratatui::layout::Rect;

fn config(flags: &[&str]) -> Config {
    build_config(Arguments::try_parse_from(["astroterm", "-d", "2025-03-01T11:00:00", "-s", "0", "-i", "Tokyo"].into_iter().chain(flags.iter().copied())).unwrap(),
        &astroterm::catalog::load_embedded_cities().unwrap()).unwrap()
}

fn tables() -> SkyTables {
    let prepared = astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
    astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap()
}

#[test]
fn controls_change_the_view_and_the_clock_and_quit_is_detected() {
    let config = config(&[]);
    let mut clock = SimulationClock::start(config.simulation.start_julian_date, 0.0);
    let mut view = config.view;
    let input = FrameInput { controls: vec![Control::ZoomIn, Control::TogglePause], resized: false };
    assert!(!stop_on_quit(&input));
    apply_frame_controls(&input, &config.view, &mut view, &mut clock);
    assert!(view.fov_degrees < config.view.fov_degrees && clock.is_paused());
    apply_frame_controls(&FrameInput { controls: vec![Control::ResetView], resized: false }, &config.view, &mut view, &mut clock);
    assert_eq!(view, config.view);
    assert!(stop_on_quit(&FrameInput { controls: vec![Control::PanLeft, Control::Quit], resized: false }));
    assert_eq!(resolve_frame_time(true, J2000, &clock).utc, J2000);                                  // single-frame mode shows the requested epoch
    assert_eq!(resolve_frame_time(false, J2000, &clock).utc, config.simulation.start_julian_date);
}

#[test]
fn the_panel_is_empty_when_off_and_lists_time_counts_display_and_timings_when_on() {
    let tables = tables();
    let time = FrameTime::from_utc(config(&[]).simulation.start_julian_date);
    let clock = SimulationClock::start(time.utc, 0.0);
    let view = View { center: ViewCenter::Zenith, fov_degrees: 360.0, projection: ProjectionKind::Equidistant };     // the whole sky, as chunk 10 tests it
    let observation = observe(&config(&[]).simulation.observer, false, &view, time, &mut astroterm::timing::StepTimes::default());
    let solar_system = simulate_solar_system(&tables.bodies, time, &mut astroterm::timing::StepTimes::default());
    let selected = select_regions(&tables.stars, &observation, &mut astroterm::timing::StepTimes::default());
    let mut caches = RegionCaches::default();
    let refreshed = refresh_regions(&mut caches, &tables, &selected, time, &view, &config(&[]).render, &mut astroterm::timing::StepTimes::default());
    let display = DisplayInfo { protocol: ProtocolType::Kitty, tmux: false, compression: CompressionSupport::Unknown, shared_memory: false, screen: Rect::new(0, 0, 100, 40), font: (10, 20), image: (1000, 800), text_cell: (9, 17) };
    let frame = PanelFrame { time, clock: &clock, solar_system: &solar_system, selected: &selected, caches: &caches, refreshed, display };
    let mut times = StepTimes::default();
    times.measure("Projection", || {});

    let off = config(&[]);
    let mut panel = Panel::new(&off, &tables);
    assert!(panel.lines(&frame, &off.simulation.observer, &view, &tables, &times).is_empty());

    let on = config(&["-m", "--debug-frametimes"]);
    let mut panel = Panel::new(&on, &tables);
    let lines = panel.lines(&frame, &on.simulation.observer, &view, &tables, &times).to_vec();
    let labels: Vec<&str> = lines.iter().map(|field| field.label.as_str()).collect();
    assert!(labels.iter().any(|label| label.starts_with("Date (")), "{labels:?}");
    for label in ["Lunar Phase", "Stars loaded", "In selected regions", "Regions selected", "Tile memory", "Graphics", "Transport", "Compression", "Groups refreshed", "Frame Time"] {
        assert!(labels.contains(&label), "{label} missing from {labels:?}");
    }
    let value = |label: &str| lines.iter().find(|field| field.label == label).unwrap().value.clone();
    assert_eq!(value("Stars loaded"), format!("{} stars", format_star_count(tables.stars.len() + tables.constellations.len())));
    assert!(value("Regions selected").ends_with("(whole sky)"));
    assert!(value("Graphics").contains("1000×800"));
    assert!(value("Groups refreshed").starts_with(&format!("{} simulated", refreshed.simulated)));
    assert!(labels.iter().any(|label| label.trim() == "Projection"));                                // the step timings follow the metadata

    for (flags, expected) in [(&["-m"][..], "Pisces"), (&["-m", "-u"][..], "Pisces ♓")] {
        let config = config(flags);
        let mut panel = Panel::new(&config, &tables);
        let fields = panel.lines(&frame, &config.simulation.observer, &view, &tables, &times);
        assert_eq!(fields.iter().find(|field| field.label == "Zodiac").unwrap().value, expected);
    }
}
