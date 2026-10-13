//! Small subprocess probe for scripts/checks/terminal.py and pixels.py. Never used by the normal application.

use astroterm::terminal::{detect_cell_aspect_ratio, open_terminal_session};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("aspect") => println!("aspect={:.6}", detect_cell_aspect_ratio()),
        Some("panic") => {
            let _session = open_terminal_session().expect("open probe terminal");
            panic!("intentional terminal restoration probe");
        }
        Some("pixel-panic") => {
            // one frame through the seven steps on a forced Kitty display, then a panic inside a synchronized update
            use astroterm::{cli::{Arguments, build_config}, present::{Display, present}, observation::observe, points::{PointInputs, draw_points},
                projection::project_stars_to_screen, regions::{RegionCaches, refresh_regions}, selection::select_regions, solar_system::simulate_solar_system,
                astro::FrameTime};
            use clap::Parser;
            let config = build_config(Arguments::parse_from(["probe", "--graphics-protocol", "kitty"]), &[]).unwrap();
            let prepared = astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
            let tables = astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap();
            let mut display = Display::open(Some(ratatui_image::picker::ProtocolType::Kitty), config.terminal, config.text_scale).unwrap();
            let mut frame = display.new_frame();
            let mut caches = RegionCaches::default();
            let time = FrameTime::from_utc(config.simulation.start_julian_date);
            let solar_system = simulate_solar_system(&tables.bodies, time, &mut astroterm::timing::StepTimes::default());
            let observation = observe(&config.simulation.observer, config.simulation.refraction, &config.view, time, &mut astroterm::timing::StepTimes::default());
            let selected = select_regions(&tables.stars, &observation, &mut astroterm::timing::StepTimes::default());
            refresh_regions(&mut caches, &tables, &selected, time, &config.view, &config.render, &mut astroterm::timing::StepTimes::default());
            project_stars_to_screen(&mut frame, &caches, &selected, &observation, &config.view, &mut astroterm::timing::StepTimes::default());
            let inputs = PointInputs { solar_system: &solar_system, tables: &tables, caches: &caches, selected: &selected, panel: &[], notice: None };
            draw_points(&mut frame, &inputs, &observation, &config.view, &config.render, &mut astroterm::timing::StepTimes::default());
            present(&frame, display.terminal(), &mut astroterm::timing::StepTimes::default()).unwrap();
            crossterm::execute!(std::io::stdout(), crossterm::terminal::BeginSynchronizedUpdate).unwrap();
            panic!("intentional pixel restoration probe");
        }
        _ => panic!("expected aspect or panic"),
    }
}
