//! astroterm: stars, planets, constellations and more, rendered in the terminal.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

use astroterm::astro::{Observer, SimulationClock, compute_precession_matrix, greenwich_mean_sidereal_time};
use astroterm::catalog::{load_embedded_catalog, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::controls::{apply_control, key_to_control};
use astroterm::sky::{Sky, refract_sky_positions, update_moon, update_planet_positions, update_star_positions};
use astroterm::terminal::{TerminalRenderer, open_terminal_renderer, poll_frame_input};

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

    // build the sky from the embedded catalogs
    let mut sky = match load_embedded_catalog() {
        Ok(catalog) => Sky::from_catalog(&catalog),
        Err(error) => return report_failure(error),
    };

    // render in the terminal, which is restored before any error is reported
    let result = open_terminal_renderer(config.render, config.aspect_ratio, config.metadata)
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
    let mut clock = SimulationClock::start(config.start_julian_date, config.speed);

    loop {
        let frame_start = Instant::now();

        // handle key presses and terminal resizes
        let input = poll_frame_input(config.quit_on_any_key)?;
        if input.quit {
            return Ok(());
        }
        if input.resized {
            renderer.fit_to_terminal()?;
        }
        for control in input.keys.iter().filter_map(key_to_control) {
            apply_control(control, &mut view, &mut clock, &config.view);
        }

        // move the sky to the current simulation time
        let julian_date = clock.julian_date();
        update_sky_positions(sky, julian_date, &config.observer);
        if config.refraction {
            refract_sky_positions(sky);
        }

        // render it
        renderer.render_frame(sky, &view, julian_date, &clock, &config.observer)?;

        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed())); // wait for the rest of the frame
    }
}

/// Move every object to its apparent position for the observer at `julian_date`.
fn update_sky_positions(sky: &mut Sky, julian_date: f64, observer: &Observer) {
    // Earth's orientation at the date: its rotation, and how far its axis has precessed since J2000
    let sidereal_time = greenwich_mean_sidereal_time(julian_date);
    let precession = compute_precession_matrix(julian_date);

    // positions of all objects
    update_star_positions(&mut sky.stars, julian_date, sidereal_time, &precession, observer);
    update_planet_positions(&mut sky.planets, julian_date, sidereal_time, &precession, observer);
    update_moon(&mut sky.moon, julian_date, sidereal_time, observer);
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}
