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
            use astroterm::{cli::{Arguments, build_config}, present::{Display, present}, observation::observe, points::{draw_points, draw_panel_and_notices},
                projection::project_stars_to_screen, regions::refresh_regions, selection::select_regions, solar_system::simulate_solar_system};
            use clap::Parser;
            let config = build_config(Arguments::parse_from(["probe", "--graphics-protocol", "kitty"]), &[]).unwrap();
            let prepared = astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
            let mut tables = astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap();
            let mut display = Display::open(Some(ratatui_image::picker::ProtocolType::Kitty), config.terminal, config.text_scale).unwrap();
            let mut frame = display.new_frame();
            let time = config.simulation.start_time_ms;
            let solar_system = simulate_solar_system(&tables.solar_system_sun, &tables.solar_system_planets, &tables.solar_system_moons, time, &mut astroterm::time::StepTimes::default());
            let observation = observe(&config.simulation.observer, config.simulation.refraction, &config.view, time, solar_system.earth, &mut astroterm::time::StepTimes::default());
            let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut astroterm::time::StepTimes::default());
            refresh_regions(
                &mut tables.star_regions, &tables.constellation_stars,
                &mut tables.constellation_simulation, &mut tables.constellation_last_update_ms,
                &mut tables.star_opacities, &mut tables.tile_bytes,
                &selected.regions, time, config.view.fov_degrees, config.render.magnitude_threshold,
                &mut astroterm::time::StepTimes::default(),
            );
            let mut projection = astroterm::projection::ProjectionExecutor::default();
            let projected_stars = project_stars_to_screen(frame.drawing_area_width_and_height, &tables.star_regions, &selected.regions, &observation, &config.view, &mut projection, &mut astroterm::time::StepTimes::default());
            frame.insert_projected_stars(projected_stars);
            let drawn = draw_points(
                &mut frame, &solar_system,
                &tables.solar_system_sun, &tables.solar_system_planets, &tables.solar_system_moons,
                &tables.constellation_stars, &tables.constellation_simulation, &tables.constellation_figures,
                &tables.star_regions, &tables.names, &tables.star_opacities, &tables.clipping,
                &selected.regions, None,
                &observation, &config.view, &config.render, &mut astroterm::time::StepTimes::default(),
            );
            draw_panel_and_notices(&mut frame, &[], &drawn.notices);
            present(&frame, display.terminal(), &mut astroterm::time::StepTimes::default()).unwrap();
            crossterm::execute!(std::io::stdout(), crossterm::terminal::BeginSynchronizedUpdate).unwrap();
            panic!("intentional pixel restoration probe");
        }
        _ => panic!("expected aspect or panic"),
    }
}
