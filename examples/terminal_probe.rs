//! Small subprocess probe for scripts/checks/terminal.py. Never used by the normal application.

use astroterm::terminal::{detect_cell_aspect_ratio, open_terminal_session};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("aspect") => println!("aspect={:.6}", detect_cell_aspect_ratio()),
        Some("panic") => {
            let _session = open_terminal_session().expect("open probe terminal");
            panic!("intentional terminal restoration probe");
        }
        Some("pixel-panic") => {
            use astroterm::{
                cli::{Arguments, build_config},
                terminal::Renderer,
            };
            use clap::Parser;
            let arguments = Arguments::parse_from(["probe", "--renderer", "pixels", "--graphics-protocol", "kitty"]);
            let config = build_config(arguments, &[]).unwrap();
            let (mut renderer, mut rendering) = Renderer::open(
                config.renderer,
                config.graphics_protocol,
                config.render,
                config.terminal,
                config.text_scale,
            )
            .unwrap();
            let sky = astroterm::sky::create_sky_from_catalog(&astroterm::catalog::load_embedded_catalog().unwrap()).unwrap();
            let projected_data = astroterm::projection::project_sky(&sky, &config.view, renderer.viewport(&rendering));
    let projected = projected_data.view(&sky);
            let clock = astroterm::astro::SimulationClock::start(config.simulation.start_julian_date, 0.0);
            renderer
                .render_frame(
                    &mut rendering,
                    &projected,
                    &config.view,
                    clock.julian_date(),
                    &clock,
                    &config.simulation.observer,
                    &mut astroterm::timing::StepTimes::default(),
                )
                .unwrap();
            crossterm::execute!(std::io::stdout(), crossterm::terminal::BeginSynchronizedUpdate).unwrap();
            panic!("intentional pixel restoration probe");
        }
        _ => panic!("expected aspect or panic"),
    }
}
