//! astroterm: stars, planets, constellations and more, rendered in the terminal.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

use astroterm::astro::SimulationClock;
use astroterm::catalog::{load_athyg_catalog, load_embedded_catalog, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::controls::{Control, apply_control};
use astroterm::projection::project_sky;
use astroterm::sky::{FrameTime, SimulationState, Sky, SkyCatalog, observe_sky, prepare_observer, update_simulation};
use astroterm::terminal::{TerminalRenderer, open_terminal_renderer, poll_frame_input};
use astroterm::timing::StepTimes;

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {
    // parse options and load the city table they may refer to
    let arguments = Arguments::parse();
    let cities = match load_embedded_cities() {
        Ok(cities) => cities,
        Err(error) => return report_failure(error),
    };

    // printing shell completions is a command of its own
    if arguments.bash_completions {
        return match write_bash_completions(&mut io::stdout().lock(), &cities) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => report_failure(error),
        };
    }

    // validate options
    let config = match build_config(arguments, &cities) {
        Ok(config) => config,
        Err(error) => return report_failure(error),
    };

    // build the sky from the embedded catalogs, or from a star dataset file
    let catalog = match &config.dataset {
        Some(path) => load_athyg_catalog(path),
        None => load_embedded_catalog(),
    };
    let mut sky = match catalog {
        Ok(catalog) => Sky::new(Arc::new(SkyCatalog::from_catalog(&catalog))),
        Err(error) => return report_failure(error),
    };

    // render in the terminal, which is restored before any error is reported
    let result = open_terminal_renderer(config.render, config.terminal)
        .and_then(|mut renderer| run_render_loop(&config, &mut sky, &mut renderer));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
fn run_render_loop(config: &Config, sky: &mut Sky, renderer: &mut TerminalRenderer) -> io::Result<()> {
    // start from the configured view and time
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));
    let mut view = config.view;
    let simulation = &config.simulation;
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);
    let mut step_times = StepTimes::default(); // how long each step of a frame takes, for --debug-frametimes

    let mut simulation_state = SimulationState::default();

    loop {
        let frame_start = Instant::now();

        // handle key presses and terminal resizes
        let input = poll_frame_input(config.terminal.quit_on_any_key)?;
        if input.controls.contains(&Control::Quit) {
            return Ok(());
        }
        if input.resized {
            renderer.fit_to_terminal()?;
        }
        for &control in &input.controls {
            apply_control(control, &mut view, &mut clock, &config.view);
        }

        // refresh only model samples whose validity no longer covers this frame
        let time = FrameTime::from_utc(clock.julian_date());
        step_times
            .measure_steps("Simulation", |steps| {
                update_simulation(&mut simulation_state, time, &[], steps)
            })
            .map_err(io::Error::other)?;

        // observe at the current epoch, with exact body spin and observer corrections
        step_times
            .measure_steps("Observation", |steps| {
                let observer = prepare_observer(&simulation_state, time, simulation.observer)?;
                observe_sky(
                    &simulation_state,
                    &observer,
                    config.render.magnitude_threshold,
                    simulation.refraction,
                    sky,
                    steps,
                )
            })
            .map_err(io::Error::other)?;

        // project the immutable observed sky for this camera, then render prepared screen geometry
        let projected = step_times.measure("Projection", || project_sky(sky, &view, renderer.viewport()));
        renderer.render_frame(
            &projected,
            &view,
            time.utc,
            &clock,
            &simulation.observer,
            &mut step_times,
        )?;

        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed())); // wait for the rest of the frame
    }
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}
