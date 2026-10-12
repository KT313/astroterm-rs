//! Optional trace descriptions, table dumps and reporting after terminal cleanup.
mod benchmark;
pub(crate) use benchmark::{BenchmarkSession, open_frame_benchmark, begin_benchmark_capture, prepare_benchmark_frame,
    validate_benchmark_time, complete_benchmark_frame, finish_benchmark_report};
use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::time::Instant;
use astroterm::cli::Arguments;
use astroterm::model::{Config, FrameTime};
use astroterm::state::{ApplicationState};
use astroterm::timing::StepTimes;

/// Capture startup timings for the requested diagnostic mode.
pub(crate) fn start_step_times(arguments: &Arguments) -> StepTimes {
    StepTimes::with_trace(arguments.debug_singleframe || arguments.debug_frames.is_some())
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

    if config.debug_singleframe && !failed
        && let Err(error) = write_run_report(config, times, output)
    {
        let prefix = if failed { "Additional diagnostic report error" } else { "ERROR" };
        let _ = writeln!(errors, "{prefix}: {error}");
        return ExitCode::FAILURE;
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Serialize the owned trace only, after terminal cleanup or a startup failure.
fn write_run_report(config: &Config, times: &StepTimes, output: &mut impl io::Write) -> io::Result<()> {
    if config.debug_singleframe { times.trace().expect("single-frame trace initialized").write_report(output)?; }
    output.flush()
}

/// Print an error and return a failing exit code.
pub(super) fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}

/// Start measuring before opening this frame's timing records.
pub(crate) fn begin_frame_diagnostics(times: &mut StepTimes) -> Instant {
    let frame_start = Instant::now();
    times.begin_frame();
    frame_start
}

/// Record the duration of a successful frame, including reuse of an already displayed image, through presentation.
pub(crate) fn finish_frame_diagnostics(config: &Config, frame_start: Instant, time: FrameTime, outcome: astroterm::model::RenderOutcome, times: &mut StepTimes) -> f64 {
    if !(config.debug_singleframe || config.frame_benchmark.is_some()) { return 0.0; }
    let elapsed = frame_start.elapsed().as_secs_f64();
    times.describe(if outcome.was_presented() { "Present" } else { "Presentation decision" }, || format!(
        "frame elapsed through presentation={:.3} ms; UTC JD={:.9}; includes in-frame diagnostics; sleep excluded",
        elapsed * 1000.0, time.utc
    ));
    elapsed
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
