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
use astroterm::projection::ProjectionCache;
use astroterm::sky::{FrameTime, ObservationCache, SimulationState, Sky, update_simulation};
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {
    // parse options and load the city table they may refer to
    let arguments = Arguments::parse();
    let mut step_times = StepTimes::with_trace(arguments.debug_singleframe);
    let cities = match step_times.measure("City loading", load_embedded_cities) {
        Ok(cities) => cities,
        Err(error) => return report_failure(error),
    };

    step_times.describe("City loading", || format!("cities loaded={}", cities.len()));

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
    let catalog = step_times.measure_steps("Dataset loading", |times| {
        astroterm::sky::cache::load_sky_catalog_with_times(
            config.dataset.as_ref(),
            &directories,
            &mut io::stderr().lock(),
            times,
        )
    });
    let mut sky = match catalog {
        Ok(catalog) => Sky::new(Arc::new(catalog)),
        Err(error) => return report_failure(error),
    };

    step_times.describe("Dataset loading", || format!(
        "output stars={}; constellation figures={}; unique endpoints={}; always-checked stars={}; tangential fallbacks={}; mapped={}",
        sky.catalog.stars.len(), sky.catalog.constellations.len(), sky.catalog.endpoint_indices.len(),
        sky.catalog.always_checked.len(), sky.catalog.singular_count, sky.catalog.stars.is_mapped(),
    ));

    eprintln!(
        "Catalog: {} stars use tangential motion after near-collision checks.",
        sky.catalog.singular_count
    );

    // render in the terminal, which is restored before any error is reported
    let result = step_times
        .measure("Terminal setup", || {
            Renderer::open(
                config.renderer,
                config.graphics_protocol,
                config.render,
                config.terminal,
                config.text_scale,
            )
        })
        .and_then(|mut renderer| {
            step_times.describe("Terminal setup", || format!("renderer={:?}; projection viewport={}x{}; metadata={}; frame-time panel={}; runtime cache enabled={}", config.renderer, renderer.viewport().width, renderer.viewport().height, config.terminal.metadata_panel, config.terminal.frame_times, config.cache.enabled));
            run_render_loop(&config, &mut sky, &mut renderer, &mut step_times)
        });
    if result.is_ok()
        && config.debug_singleframe
        && let Err(error) = step_times.trace().unwrap().write_report(&mut io::stdout().lock())
    {
        return report_failure(error);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
fn run_render_loop(
    config: &Config,
    sky: &mut Sky,
    renderer: &mut Renderer,
    step_times: &mut StepTimes,
) -> io::Result<()> {
    // start from the configured view and time
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));
    let mut view = config.view;
    let simulation = &config.simulation;
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);
    step_times.reset_frame_timings();

    let mut simulation_state = SimulationState::default();
    simulation_state.configure_cache(&config.cache);
    let mut observation_cache = ObservationCache::new(config.cache.clone());
    let mut projection_cache = ProjectionCache::new(config.cache.clone());
    renderer.configure_cache(&config.cache);

    loop {
        let frame_start = Instant::now();
        step_times.begin_frame();

        // handle key presses and terminal resizes
        let input = poll_frame_input(config.terminal.quit_on_any_key)?;
        if input.controls.contains(&Control::Quit) {
            return Ok(());
        }
        let previous_view = view;
        if input.resized {
            renderer.fit_to_terminal()?;
        }
        for &control in &input.controls {
            apply_control(control, &mut view, &mut clock, &config.view);
        }

        if view != previous_view {
            observation_cache.invalidate_view();
            projection_cache.invalidate_view();
        } else if input.resized {
            projection_cache.invalidate_view();
        }
        simulation_state.begin_frame();

        // refresh only model samples whose validity no longer covers this frame
        let time = FrameTime::from_utc(if config.debug_singleframe {
            simulation.start_julian_date // exact requested epoch makes single-frame comparisons reproducible
        } else {
            clock.julian_date()
        });
        step_times
            .measure_steps("Simulation", |steps| {
                update_simulation(&mut simulation_state, time, &[], steps)
            })
            .map_err(io::Error::other)?;

        // observe at the current epoch, with exact body spin and observer corrections
        step_times
            .measure_steps("Observation", |steps| {
                let mut observer = steps.measure("Observer geometry", || {
                    observation_cache.prepare_observer(&simulation_state, time, simulation.observer)
                })?;
                steps.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, simulation.observer.latitude, simulation.observer.longitude));
                steps.measure_steps("Light-time sampling", |steps| {
                    observation_cache.prepare_light_time(&mut simulation_state, &mut observer, steps)
                })?;

                steps.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));

                for name in ["Observer geometry", "Light-time sampling"] {
                    steps.describe(name, || format!("cache={:?}", observation_cache.reports().into_iter().find(|r| r.name == name).unwrap()));
                }

                // observation.rs owns the timed filtering, motion, aberration, rotation and refraction passes
                observation_cache.observe(
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
            projection_cache.project(sky, &view, renderer.viewport(), time.tt, steps)
        });
        renderer.set_cache_diagnostics(observation_cache.stats(), projection_cache.stats());
        renderer.render_frame(&projected, &view, time.utc, &clock, &simulation.observer, step_times)?;

        if config.debug_singleframe {
            step_times.describe("Present", || {
                format!(
                    "frame elapsed including diagnostic bookkeeping={:.3} ms; UTC JD={:.9}; sleep excluded",
                    frame_start.elapsed().as_secs_f64() * 1000.0,
                    time.utc
                )
            });
            return Ok(());
        }

        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed())); // wait for the rest of the frame
    }
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}
