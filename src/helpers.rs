//! Application startup, frame-stage details, terminal lifetime, diagnostics and exit reporting.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use astroterm::astro::SimulationClock;
use astroterm::catalog::{City, datasets::DatasetDirectories, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::controls::apply_control;
use astroterm::projection::{ProjectedSky, ProjectionCache, View};
use astroterm::sky::{FrameTime, ObservationCache, SimulationState, Sky, update_simulation};
use astroterm::terminal::{FrameInput, Renderer};
use astroterm::timing::StepTimes;

use crate::pipeline::run_render_loop;

/// Load the embedded city table, recording diagnostics and reporting any failure.
pub(super) fn load_cities(step_times: &mut StepTimes) -> Result<Vec<City>, ExitCode> {
    let cities = match step_times.measure("City loading", load_embedded_cities) {
        Ok(cities) => cities,
        Err(error) => return Err(report_failure(error)),
    };
    step_times.describe("City loading", || format!("cities loaded={}", cities.len()));
    Ok(cities)
}

/// Print Bash completions without entering the rendering pipeline.
pub(super) fn print_bash_completions(cities: &[City]) -> ExitCode {
    match write_bash_completions(&mut io::stdout().lock(), cities) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Build the runtime configuration and report invalid arguments before returning failure.
pub(super) fn validate_arguments(arguments: Arguments, cities: &[City]) -> Result<Config, ExitCode> {
    build_config(arguments, cities).map_err(report_failure)
}

/// Load the selected catalog and record its startup diagnostics.
pub(super) fn load_catalog_sky(
    config: &Config,
    directories: &DatasetDirectories,
    step_times: &mut StepTimes,
) -> Result<Sky, ExitCode> {
    let catalog = step_times.measure_steps("Dataset loading", |times| {
        astroterm::sky::cache::load_sky_catalog_with_times(
            config.dataset.as_ref(),
            directories,
            &mut io::stderr().lock(),
            times,
        )
    });
    let sky = match catalog {
        Ok(catalog) => Sky::new(Arc::new(catalog)),
        Err(error) => return Err(report_failure(error)),
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

    Ok(sky)
}

/// Open the renderer and restore the terminal before returning the render-loop result.
pub(super) fn render_in_terminal(config: &Config, sky: &mut Sky, step_times: &mut StepTimes) -> io::Result<()> {
    step_times
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
            run_render_loop(config, sky, &mut renderer, step_times)
        })
}

/// Print a successful single-frame trace or report the rendering failure after terminal cleanup.
pub(super) fn finish_rendering(result: io::Result<()>, config: &Config, step_times: &StepTimes) -> ExitCode {
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

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}

/// Apply a frame's controls and invalidate dependent caches after any view change or resize.
/// The caller checks for quit first and retains ownership of the input until the frame ends.
pub(super) fn apply_frame_controls(
    input: &FrameInput, config: &Config, view: &mut View, clock: &mut SimulationClock,
    renderer: &mut Renderer, observation_cache: &mut ObservationCache, projection_cache: &mut ProjectionCache,
) -> io::Result<()> {
    let previous_view = *view;
    if input.resized { renderer.fit_to_terminal()?; }
    for &control in &input.controls {
        apply_control(control, view, clock, &config.view);
    }

    if *view != previous_view {
        observation_cache.invalidate_view();
        projection_cache.invalidate_view();
    } else if input.resized {
        projection_cache.invalidate_view();
    }
    Ok(())
}

/// Use the exact requested epoch for single-frame diagnostics, otherwise read the running clock.
pub(super) fn resolve_frame_time(config: &Config, clock: &SimulationClock) -> FrameTime {
    FrameTime::from_utc(if config.debug_singleframe {
        config.simulation.start_julian_date
    } else {
        clock.julian_date()
    })
}

/// Refresh model samples within the existing Simulation timer and preserve its error conversion.
pub(super) fn simulate_frame(simulation_state: &mut SimulationState, time: FrameTime, step_times: &mut StepTimes) -> io::Result<()> {
    step_times
        .measure_steps("Simulation", |steps| {
            update_simulation(simulation_state, time, &[], steps)
        })
        .map_err(io::Error::other)
}

/// Prepare observer/emission samples and run the ordered observation passes within their existing timers.
pub(super) fn observe_frame(
    config: &Config, view: &View, simulation_state: &mut SimulationState,
    observation_cache: &mut ObservationCache, sky: &mut Sky, time: FrameTime, step_times: &mut StepTimes,
) -> io::Result<()> {
    let simulation = &config.simulation;
    step_times
        .measure_steps("Observation", |steps| {
            let mut observer = steps.measure("Observer geometry", || {
                observation_cache.prepare_observer(simulation_state, time, simulation.observer)
            })?;
            steps.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, simulation.observer.latitude, simulation.observer.longitude));
            steps.measure_steps("Light-time sampling", |steps| {
                observation_cache.prepare_light_time(simulation_state, &mut observer, steps)
            })?;

            steps.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));

            for name in ["Observer geometry", "Light-time sampling"] {
                steps.describe(name, || format!("cache={:?}", observation_cache.reports().into_iter().find(|r| r.name == name).unwrap()));
            }

            // observation.rs owns the timed filtering, motion, aberration, rotation and refraction passes
            observation_cache.observe(
                simulation_state,
                &observer,
                config.render.magnitude_threshold,
                simulation.refraction,
                view.sky_region(),
                sky,
                steps,
            )
        })
        .map_err(io::Error::other)
}

/// Project the observed sky for the current viewport within the existing Projection timer.
pub(super) fn project_frame<'a>(
    sky: &'a Sky, view: &View, renderer: &Renderer, time: FrameTime,
    projection_cache: &mut ProjectionCache, step_times: &mut StepTimes,
) -> ProjectedSky<'a> {
    step_times.measure_steps("Projection", |steps| {
        projection_cache.project(sky, view, renderer.viewport(), time.tt, steps)
    })
}

/// Record the completed single frame when requested, returning whether the loop should stop before sleeping.
pub(super) fn record_single_frame_if_requested(config: &Config, frame_start: Instant, time: FrameTime, step_times: &mut StepTimes) -> bool {
    if !config.debug_singleframe { return false; }
    step_times.describe("Present", || {
        format!(
            "frame elapsed including diagnostic bookkeeping={:.3} ms; UTC JD={:.9}; sleep excluded",
            frame_start.elapsed().as_secs_f64() * 1000.0,
            time.utc
        )
    });
    true
}
