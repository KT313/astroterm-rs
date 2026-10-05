//! Application startup, frame-stage details, terminal lifetime, diagnostics and exit reporting.

use astroterm::state::{ObservationCache, ProjectionCache, SimulationState, RenderingState};
use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use astroterm::astro::SimulationClock;
use astroterm::catalog::{City, datasets::DatasetDirectories, load_embedded_cities};
use astroterm::cli::{Arguments, build_config, write_bash_completions};
use astroterm::model::config::Config;
use astroterm::controls::apply_control;
use astroterm::model::projection::View;
use astroterm::model::Sky;
use astroterm::model::simulation::FrameTime;
use astroterm::sky::update_simulation;
use astroterm::terminal::{FrameInput, Renderer};
use astroterm::timing::StepTimes;
use astroterm::state::ApplicationState;

use crate::pipeline::run_render_loop;

/// Capture startup timings for either diagnostic mode; memory collection still waits for validated options.
pub(super) fn start_step_times(arguments: &Arguments) -> StepTimes {
    let memory_requested = cfg!(feature = "memory-diagnostics") && arguments.debug_memory;
    StepTimes::with_trace(arguments.debug_singleframe || memory_requested)
}

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
        Err(error) => return Err(finish_requested_report(Err(io::Error::other(error)), config, step_times,
            &mut io::stdout().lock(), &mut io::stderr().lock())),
    };

    #[cfg(feature = "memory-diagnostics")]
    step_times.record_memory(step_times.last_memory_step(), || {
        use astroterm::timing::memory::{BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
        let shape = BufferShape { len: Some(sky.catalog.stars.len()), ..BufferShape::unknown(IndexDomain::Catalog) };
        MemoryEvent::operation(BufferId::CatalogStars, if sky.catalog.stars.is_mapped() { Operation::Map } else { Operation::Build }, None, Some(shape), shape.len, None)
    });

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
pub(super) fn render_in_terminal(state: &mut ApplicationState) -> io::Result<()> {
    let renderer = state.timings.measure("Terminal setup", || {
        let config = &state.config;
        Renderer::open(config.renderer, config.graphics_protocol, config.render, config.terminal, config.text_scale)
    });
    renderer.and_then(|(mut renderer, rendering)| {
        state.run.rendering = rendering;
        let config = &state.config;
        state.timings.describe("Terminal setup", || format!("renderer={:?}; projection viewport={}x{}; metadata={}; frame-time panel={}; runtime cache enabled={}", config.renderer, renderer.viewport(&state.run.rendering).width, renderer.viewport(&state.run.rendering).height, config.terminal.metadata_panel, config.terminal.frame_times, config.cache.enabled));
        let result = run_render_loop(state, &mut renderer);
        #[cfg(feature = "memory-diagnostics")]
        capture_failed_frame_memory(state, &renderer, &result);
        result
    })
}

/// Report diagnostics and the original rendering result after the terminal helper has dropped its guard.
pub(super) fn finish_rendering(result: io::Result<()>, state: &ApplicationState) -> ExitCode {
    finish_requested_report(result, &state.config, &state.timings, &mut io::stdout().lock(), &mut io::stderr().lock())
}

/// Keep an operational error primary even if the separate diagnostic writer also fails.
fn finish_requested_report(result: io::Result<()>, config: &Config, times: &StepTimes,
    output: &mut impl io::Write, errors: &mut impl io::Write) -> ExitCode {
    let failed = result.is_err();
    if let Err(error) = result { let _ = writeln!(errors, "ERROR: {error}"); }

    let memory_requested = cfg!(feature = "memory-diagnostics") && config.debug_memory;
    if (memory_requested || (config.debug_singleframe && !failed))
        && let Err(error) = write_run_report(config, times, output)
    {
        let prefix = if failed { "Additional diagnostic report error" } else { "ERROR" };
        let _ = writeln!(errors, "{prefix}: {error}");
        return ExitCode::FAILURE;
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Serialize owned diagnostic snapshots only, after terminal cleanup or a startup failure.
fn write_run_report(config: &Config, times: &StepTimes, output: &mut impl io::Write) -> io::Result<()> {
    #[cfg(feature = "memory-diagnostics")]
    if config.debug_memory {
        let report_start = Instant::now();
        writeln!(output, "Memory report mode: {}", if config.debug_singleframe { "single frame" } else { "continuous run" })?;
        times.write_memory_run_report(output)?;
        for snapshot in times.memory_inventories() { astroterm::state::memory::write_inventory(snapshot, output)?; }
        writeln!(output, "Report formatting/output before this line: {:.3} ms (after cleanup; final flush excluded)", report_start.elapsed().as_secs_f64() * 1000.0)?;
        return output.flush();
    }
    if config.debug_singleframe { times.trace().expect("single-frame trace initialized").write_report(output)?; }
    output.flush()
}

/// Preserve partial-frame evidence before the renderer guard drops; setup failures have no pending frame.
#[cfg(feature = "memory-diagnostics")]
fn capture_failed_frame_memory(state: &mut ApplicationState, renderer: &Renderer, result: &io::Result<()>) {
    if result.is_ok() { return; }
    let Some(run) = state.timings.memory_run() else { return; };
    if !run.frame_active { return; }
    let tt = run.current_time.map(|time| time.1);
    capture_memory(&state.config, &state.catalog, &state.run, renderer, &mut state.timings, "After incomplete frame", tt);
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}

#[allow(clippy::too_many_arguments)]
/// Apply a frame's controls and invalidate dependent caches after any view change or resize.
/// The caller checks for quit first and retains ownership of the input until the frame ends.
pub(super) fn apply_frame_controls(
    input: &FrameInput, config: &Config, view: &mut View, clock: &mut SimulationClock,
    renderer: &mut Renderer, rendering: &mut RenderingState, observation_cache: &mut ObservationCache, projection_cache: &mut ProjectionCache,
) -> io::Result<()> {
    let previous_view = *view;
    if input.resized { renderer.fit_to_terminal(rendering)?; }
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
            let mut observer = astroterm::sky::prepare_cached_observer_with_times(observation_cache, simulation_state, time, simulation.observer, steps)?;
            steps.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, simulation.observer.latitude, simulation.observer.longitude));
            steps.measure_steps("Light-time sampling", |steps| {
                astroterm::sky::prepare_cached_light_time(observation_cache, simulation_state, &mut observer, steps)
            })?;

            steps.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));

            steps.describe("Observer geometry", || format!("cache={:?}", observation_cache.observer_report()));
            steps.describe("Light-time sampling", || format!("cache={:?}", observation_cache.light_time_report()));

            // observation.rs owns the timed filtering, motion, aberration, rotation and refraction passes
            astroterm::sky::observe_cached_sky(observation_cache, simulation_state,
                &observer,
                config.render.magnitude_threshold,
                simulation.refraction,
                astroterm::projection::select_view_region(view),
                sky,
                steps)
        })
        .map_err(io::Error::other)
}

/// Project the observed sky for the current viewport within the existing Projection timer.
pub(super) fn project_frame(
    sky: &Sky, view: &View, viewport: astroterm::model::projection::ProjectionViewport, time: FrameTime,
    projection_cache: &mut ProjectionCache, step_times: &mut StepTimes,
) {
    step_times.measure_steps("Projection", |steps| {
        astroterm::projection::project_cached_sky(projection_cache, sky, view, viewport, time.tt, steps);
    })
}

/// Cancel the empty diagnostic frame on a quit key without losing the last successfully presented frame.
pub(super) fn stop_on_quit(input: &FrameInput, times: &mut StepTimes) -> bool {
    if !input.controls.contains(&astroterm::controls::Control::Quit) { return false; }
    times.cancel_memory_frame();
    true
}

/// Sample duration through presentation before inspecting memory; ordinary single-frame output stays compatible.
pub(super) fn record_frame_duration(config: &Config, frame_start: Instant, time: FrameTime, step_times: &mut StepTimes) -> f64 {
    if !(config.debug_singleframe || (cfg!(feature = "memory-diagnostics") && config.debug_memory)) { return 0.0; }
    let elapsed = frame_start.elapsed().as_secs_f64();
    step_times.describe("Present", || format!(
        "frame elapsed through presentation={:.3} ms; UTC JD={:.9}; includes in-frame diagnostics; final memory capture and sleep excluded",
        elapsed * 1000.0, time.utc
    ));
    elapsed
}

/// Capture state-owned working buffers and the separately scoped terminal writer while both are alive.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "memory-diagnostics"), allow(unused_variables))]
#[inline]
pub(super) fn capture_memory(config: &Config, catalog: &Arc<astroterm::model::SkyCatalog>, run: &astroterm::state::RunState, renderer: &Renderer, times: &mut StepTimes, label: &'static str, tt: Option<f64>) {
    #[cfg(feature = "memory-diagnostics")]
    if config.debug_memory { times.capture_memory(|times| astroterm::state::memory::capture_run_inventory(config, catalog, run, renderer, times, label, tt)); }
}

/// Record the actual read-only frame view, without creating another view or copying its geometry.
#[inline]
pub(super) fn record_projected_memory(times: &mut StepTimes, projected: &astroterm::model::projection::ProjectedSky<'_>) {
    use astroterm::timing::memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent};
    times.record_memory(times.last_memory_step(), || {
        let shape = BufferShape { len: Some(projected.stars.len()), ..BufferShape::unknown(IndexDomain::Visible) };
        MemoryEvent::borrow(BufferId::ProjectedView, Access::ReadOnly, shape)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    struct FailingWriter { fail_flush: bool }
    impl io::Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_flush { Ok(bytes.len()) } else { Err(io::Error::other("report write failed")) }
        }
        fn flush(&mut self) -> io::Result<()> { Err(io::Error::other("report flush failed")) }
    }

    fn report_config(flags: &[&str]) -> Config {
        build_config(Arguments::try_parse_from(flags).unwrap(), &[]).unwrap()
    }

    #[test]
    fn plain_single_frame_keeps_existing_trace_and_failed_run_stays_unreported() {
        let config = report_config(&["astroterm", "--debug-singleframe"]);
        let mut times = StepTimes::with_trace(true);
        times.measure("Present", || ());
        let mut expected = Vec::new();
        times.trace().unwrap().write_report(&mut expected).unwrap();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        assert_eq!(finish_requested_report(Ok(()), &config, &times, &mut output, &mut errors), ExitCode::SUCCESS);
        assert_eq!(output, expected);
        assert!(errors.is_empty());

        output.clear();
        assert_eq!(finish_requested_report(Err(io::Error::other("render failed")), &config, &times,
            &mut output, &mut errors), ExitCode::FAILURE);
        assert!(output.is_empty());
        assert_eq!(String::from_utf8(errors).unwrap(), "ERROR: render failed\n");
    }

    #[test]
    fn report_write_or_flush_failure_turns_success_into_failure() {
        let config = report_config(&["astroterm", "--debug-singleframe"]);
        let times = StepTimes::with_trace(true);
        for fail_flush in [false, true] {
            let mut errors = Vec::new();
            assert_eq!(finish_requested_report(Ok(()), &config, &times, &mut FailingWriter { fail_flush },
                &mut errors), ExitCode::FAILURE);
            assert!(String::from_utf8(errors).unwrap().starts_with("ERROR: report"));
        }
    }

    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn original_operation_error_precedes_secondary_report_failure() {
        let config = report_config(&["astroterm", "--debug-memory"]);
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_run(false);
        times.measure("Dataset loading", || ());
        let mut errors = Vec::new();
        assert_eq!(finish_requested_report(Err(io::Error::other("original operation failed")), &config, &times,
            &mut FailingWriter { fail_flush: false }, &mut errors), ExitCode::FAILURE);
        assert_eq!(String::from_utf8(errors).unwrap(),
            "ERROR: original operation failed\nAdditional diagnostic report error: report write failed\n");
    }

    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn partial_failure_report_keeps_completed_frame_count_and_failed_time() {
        let config = report_config(&["astroterm", "--debug-memory"]);
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_run(true);
        times.measure("Startup", || ());
        times.begin_memory_frame();
        times.set_memory_frame_time(10.0, 10.1);
        times.measure("Present", || ());
        times.complete_memory_frame(0.01);
        times.begin_memory_frame();
        times.set_memory_frame_time(11.0, 11.1);
        times.measure("Failing simulation", || ());
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        assert_eq!(finish_requested_report(Err(io::Error::other("simulation failed")), &config, &times,
            &mut output, &mut errors), ExitCode::FAILURE);
        let report = String::from_utf8(output).unwrap();
        assert!(report.contains("Latest completed frame"));
        assert!(report.contains("Incomplete frame"));
        assert!(report.contains("Failing simulation"));
        assert_eq!(times.memory_run().unwrap().completed_frames, 1);
        assert_eq!(times.memory_run().unwrap().current_time, Some((11.0, 11.1)));
    }

    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn quit_discards_pending_frame_and_reports_zero_completed_without_partial_failure() {
        let arguments = Arguments::try_parse_from(["astroterm", "--debug-memory"]).unwrap();
        let mut times = start_step_times(&arguments);
        times.measure("City loading", || ());
        let config = build_config(arguments, &[]).unwrap();
        times.enable_memory_run(true);
        times.begin_memory_frame();
        let input = FrameInput { controls: vec![astroterm::controls::Control::Quit], resized: false };
        assert!(stop_on_quit(&input, &mut times));
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        assert_eq!(finish_requested_report(Ok(()), &config, &times, &mut output, &mut errors), ExitCode::SUCCESS);
        let report = String::from_utf8(output).unwrap();
        assert!(report.contains("City loading"));
        assert!(!report.contains("Incomplete frame"));
        assert_eq!(times.memory_run().unwrap().completed_frames, 0);
    }
}
