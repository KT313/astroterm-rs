//! Optional trace descriptions, table dumps, memory captures and reporting after terminal cleanup.
use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;
use astroterm::cli::Arguments;
use astroterm::model::{Config, FrameTime};
use astroterm::state::{ApplicationState, ObservationCache};
use astroterm::terminal::Renderer;
use astroterm::timing::StepTimes;

/// Capture startup timings for either diagnostic mode; memory collection still waits for validated options.
pub(crate) fn start_step_times(arguments: &Arguments) -> StepTimes {
    let memory_requested = cfg!(feature = "memory-diagnostics") && arguments.debug_memory;
    StepTimes::with_trace(arguments.debug_singleframe || memory_requested)
}

/// Enable bounded memory reports only after the user's options have been validated.
#[cfg_attr(not(feature = "memory-diagnostics"), allow(unused_variables))]
pub(crate) fn configure_memory_reporting(config: &Config, times: &mut StepTimes) {
    #[cfg(feature = "memory-diagnostics")]
    if config.debug_memory { times.enable_memory_run(config.cache.enabled); }
}

/// When enabled, append one named table dump and create any missing parent folders.
pub(crate) fn log_pipeline_data_if_requested(state: &ApplicationState, path: impl AsRef<std::path::Path>, section: &str) -> io::Result<()> {
    if !state.config.debug_log_data { return Ok(()); }
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
    state.log_data(Some(path), Some(section))
}

/// Report diagnostics and the original rendering result after the terminal scope has dropped its guard.
pub(crate) fn finish_rendering(result: io::Result<()>, state: &ApplicationState) -> ExitCode {
    finish_requested_report(result, &state.config, &state.timings, &mut io::stdout().lock(), &mut io::stderr().lock())
}

/// Keep an operational error primary even if the separate diagnostic writer also fails.
pub(super) fn finish_requested_report(result: io::Result<()>, config: &Config, times: &StepTimes,
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
        for snapshot in times.memory_inventories() { astroterm::state::write_inventory(snapshot, output)?; }
        writeln!(output, "Report formatting/output before this line: {:.3} ms (after cleanup; final flush excluded)", report_start.elapsed().as_secs_f64() * 1000.0)?;
        return output.flush();
    }
    if config.debug_singleframe { times.trace().expect("single-frame trace initialized").write_report(output)?; }
    output.flush()
}

/// Preserve partial-frame evidence before the renderer guard drops; setup failures have no pending frame.
#[cfg_attr(not(feature = "memory-diagnostics"), allow(unused_variables))]
pub(crate) fn capture_failed_frame_memory(state: &mut ApplicationState, renderer: &Renderer, result: &io::Result<()>) {
    #[cfg(feature = "memory-diagnostics")]
    {
        if result.is_ok() { return; }
        let Some(run) = state.timings.memory_run() else { return; };
        if !run.frame_active { return; }
        let tt = run.current_time.map(|time| time.1);
        capture_memory(&state.config, &state.persistent.catalog, &state.cache, state.preparation.as_ref(), renderer, &mut state.timings, "After incomplete frame", tt);
    }
}

/// Print an error and return a failing exit code.
pub(super) fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}

/// Keep diagnostic formatting lazy and outside the observer calculation's timer.
pub(super) fn describe_observer_geometry(time: FrameTime, site: &astroterm::astro::Observer, times: &mut StepTimes) {
    times.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, site.latitude, site.longitude));
}

/// Describe the completed light-time pass, preserving the original detail order and target steps.
pub(super) fn describe_light_time_sampling(observer: &astroterm::model::ObserverState, cache: &ObservationCache, times: &mut StepTimes) {
    times.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));
    times.describe("Observer geometry", || format!("cache={:?}", cache.observer_report()));
    times.describe("Light-time sampling", || format!("cache={:?}", cache.light_time_report()));
}

/// Sample duration through presentation before inspecting memory; ordinary single-frame output stays compatible.
pub(crate) fn record_frame_duration(config: &Config, frame_start: Instant, time: FrameTime, step_times: &mut StepTimes) -> f64 {
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
pub(crate) fn capture_memory(config: &Config, catalog: &Arc<astroterm::model::SkyCatalog>, caches: &astroterm::state::Caches, preparation: Option<&astroterm::model::CatalogPreparation>, renderer: &Renderer, times: &mut StepTimes, label: &'static str, tt: Option<f64>) {
    #[cfg(feature = "memory-diagnostics")]
    if config.debug_memory { times.capture_memory(|times| astroterm::state::capture_run_inventory(config, catalog, caches, preparation, renderer, times, label, tt)); }
}

/// Record the actual read-only frame view, without creating another view or copying its geometry.
#[inline]
pub(super) fn record_projected_memory(times: &mut StepTimes, projected: &astroterm::model::ProjectedSky<'_>) {
    use astroterm::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent};
    times.record_memory(times.last_memory_step(), || {
        let shape = BufferShape { len: Some(projected.stars.len()), ..BufferShape::unknown(IndexDomain::Visible) };
        MemoryEvent::borrow(BufferId::ProjectedView, Access::ReadOnly, shape)
    });
}

#[cfg(test)]
mod tests;
