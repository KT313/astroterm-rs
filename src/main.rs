//! astroterm: stars, planets, constellations and more, rendered in the terminal.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

use astroterm::astro::SimulationClock;
use astroterm::catalog::{datasets::DatasetDirectories, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::controls::{Control, apply_control};
use astroterm::projection::project_sky_with_times;
use astroterm::sky::{
    FrameTime, SimulationState, Sky, observe_sky, prepare_light_time_samples, prepare_observer, update_simulation,
};
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
    let directories = DatasetDirectories::for_user();
    let catalog =
        astroterm::sky::cache::load_sky_catalog(config.dataset.as_ref(), &directories, &mut io::stderr().lock());
    let mut sky = match catalog {
        Ok(catalog) => Sky::new(Arc::new(catalog)),
        Err(error) => return report_failure(error),
    };

    eprintln!(
        "Catalog: {} stars use tangential motion after near-collision checks.",
        sky.catalog.singular_count
    );

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
        step_times.begin_frame();

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
                let mut observer = steps.measure("Observer geometry", || {
                    prepare_observer(&simulation_state, time, simulation.observer)
                })?;
                steps.measure_steps("Light-time sampling", |steps| {
                    prepare_light_time_samples(&mut simulation_state, &mut observer, steps)
                })?;

                // observation.rs owns the timed filtering, motion, aberration, rotation and refraction passes
                observe_sky(
                    &simulation_state,
                    &observer,
                    config.render.magnitude_threshold,
                    simulation.refraction,
                    view.sky_region(),
                    sky,
                    steps,
                )
            })
            .map_err(io::Error::other)?;

        // project the immutable observed sky for this camera, then render prepared screen geometry
        let projected = step_times.measure_steps("Projection", |steps| {
            project_sky_with_times(sky, &view, renderer.viewport(), steps)
        });
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
