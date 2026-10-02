//! astroterm: stars, planets, constellations and more, rendered in the terminal.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;

use astroterm::astro::{Observer, SimulationClock, greenwich_mean_sidereal_time};
use astroterm::canvas::Canvas;
use astroterm::catalog::{load_embedded_catalog, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::controls::{apply_control, key_to_control};
use astroterm::projection::View;
use astroterm::scene::{
    draw_azimuthal_grid, draw_cardinal_directions, draw_constellations, draw_horizon_labels, draw_horizon_line,
    draw_metadata, draw_moon, draw_planets, draw_stars,
};
use astroterm::sky::{Sky, update_moon, update_planet_positions, update_star_positions};
use astroterm::terminal::{TerminalSession, open_terminal_session, poll_frame_input};

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

    // render inside a terminal session, which is restored before any error is reported
    let result = open_terminal_session().and_then(|mut terminal| run_render_loop(&config, &mut sky, &mut terminal));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
fn run_render_loop(config: &Config, sky: &mut Sky, terminal: &mut TerminalSession) -> io::Result<()> {
    // size the canvases to the terminal, and start from the configured view and time
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));
    let mut frame = terminal.fit_frame(config.aspect_ratio, config.metadata)?;
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
            frame = terminal.fit_frame(config.aspect_ratio, config.metadata)?;
        }
        for control in input.keys.iter().filter_map(key_to_control) {
            apply_control(control, &mut view, &mut clock, &config.view);
        }

        // move the sky to the current simulation time and draw it, with the metadata panel on top
        let julian_date = clock.julian_date();
        update_sky_positions(sky, julian_date, &config.observer);
        draw_frame(&mut frame.sky, config, &view, sky);
        if let Some(panel) = &mut frame.panel {
            let (moon_phase, unicode) = (sky.moon.phase, config.render.unicode);
            draw_metadata(panel, julian_date, &clock, moon_phase, &config.observer, &view, unicode);
        }
        terminal.present(&frame)?;

        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed())); // wait for the rest of the frame
    }
}

/// Move every object to its apparent position for the observer at `julian_date`.
fn update_sky_positions(sky: &mut Sky, julian_date: f64, observer: &Observer) {
    let sidereal_time = greenwich_mean_sidereal_time(julian_date);
    update_star_positions(&mut sky.stars, julian_date, sidereal_time, observer);
    update_planet_positions(&mut sky.planets, julian_date, sidereal_time, observer);
    update_moon(&mut sky.moon, julian_date, sidereal_time, observer);
}

/// Draw one frame of the sky as seen in `view`, back to front.
fn draw_frame(canvas: &mut Canvas, config: &Config, view: &View, sky: &Sky) {
    let options = &config.render;
    canvas.clear();

    // the horizon first in the facing view, so objects are drawn on top of it
    if view.is_facing() {
        draw_horizon_line(canvas, view, options);
    }

    // celestial objects
    draw_stars(canvas, view, options, sky);
    if config.constellations {
        draw_constellations(canvas, view, options, sky);
    }
    draw_planets(canvas, view, options, &sky.planets);
    draw_moon(canvas, view, options, sky);

    // orientation aids
    if view.is_facing() {
        draw_horizon_labels(canvas, view, options);
    } else if config.grid {
        draw_azimuthal_grid(canvas, options);
    } else {
        draw_cardinal_directions(canvas, options);
    }
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}
