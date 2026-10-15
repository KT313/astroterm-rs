//! One report writer per process; timings stay in StepTimes and are released after each serialized segment.
use astroterm::{model::{Config, FrameBenchmarkConfig}, present::{Display, Presented},
    terminal::FrameInput, time::{CaptureSteps, SimulationClock, StepTimes}};
use super::{CaptureDisplay, capture_display};
use serde::Serialize;
use std::{fs::File, io::{self, BufWriter, Write}, process::ExitCode};

pub(crate) struct BenchmarkSession<W: Write = BufWriter<File>> {
    writer: W,
    settings: FrameBenchmarkConfig,
    completed: u32,
    presented: u32,
    reused: u32,
    display: Option<CaptureDisplay>,
    write_failed: bool,
    start_time_ms: Option<i64>,
}

#[derive(Serialize)]
struct Segment<'a> { r#type: &'static str, trace_complete: bool, steps: CaptureSteps<'a> }
#[derive(Serialize)]
struct Frame<'a> {
    r#type: &'static str, index: u32, simulation_time_ms: i64, elapsed_seconds: f64,
    presentation: &'static str, display: CaptureDisplay, trace_complete: bool, steps: CaptureSteps<'a>,
}

impl<W: Write> BenchmarkSession<W> {
    fn new(writer: W, settings: FrameBenchmarkConfig) -> Self {
        Self { writer, settings, completed: 0, presented: 0, reused: 0, display: None, write_failed: false, start_time_ms: None }
    }

    fn write_record(&mut self, record: &impl Serialize) -> io::Result<()> {
        if self.write_failed { return Err(io::Error::other("benchmark report writer previously failed")); }
        let result = (|| {
            serde_json::to_writer(&mut self.writer, record).map_err(io::Error::other)?;
            self.writer.write_all(b"\n")?;
            self.writer.flush()
        })();
        self.write_failed = result.is_err();
        result
    }

    fn begin_capture(&mut self, config: &Config, display: CaptureDisplay, times: &mut StepTimes) -> io::Result<()> {
        self.write_record(&serde_json::json!({
            "type": "run", "schema_version": 2, "requested_frames": self.settings.frames,
            "step_ms": self.settings.step_ms, "start_time_ms": config.simulation.start_time_ms,
            "time_epoch": "0000-01-01T00:00:00", "time_unit": "milliseconds",
            "time_scale": "proleptic Gregorian UTC/UT, uniform days",
            "renderer": display.renderer, "graphics_protocol": display.graphics_protocol, "geometry": display.geometry,
            "transport": display.transport, "compression_support": display.compression_support, "tmux": display.tmux, "timing_panel_enabled": config.terminal.frame_times,
            "effective_config": {"view": format!("{:?}", config.view),
                "render": format!("{:?}", config.render), "observer": format!("{:?}", config.simulation.observer),
                "refraction": config.simulation.refraction, "metadata_panel": config.terminal.metadata_panel,
                "fps": config.fps, "text_scale": config.text_scale, "dataset": format!("{:?}", config.dataset)}
        }))?;
        self.write_record(&Segment { r#type: "startup", trace_complete: true, steps: times.capture_steps()? })?;
        times.clear_capture_segment()?;
        self.display = Some(display);
        self.start_time_ms = Some(config.simulation.start_time_ms);
        Ok(())
    }

    fn record_frame(&mut self, time_ms: i64, presented: Presented, elapsed: f64, display: CaptureDisplay, times: &mut StepTimes) -> io::Result<bool> {
        if self.completed >= self.settings.frames || !elapsed.is_finite() || elapsed < 0.0 { return Err(io::Error::other("invalid benchmark frame completion")); }
        let start_ms = self.start_time_ms.ok_or_else(|| io::Error::other("benchmark startup was not reported"))?;
        let scheduled_ms = i128::from(start_ms) + i128::from(self.completed) * i128::from(self.settings.step_ms);
        if i128::from(time_ms) != scheduled_ms { return Err(io::Error::other("benchmark frame date differs from its scheduled millisecond timestamp")); }
        self.completed += 1;                                                                  // successful rendering counts even when reporting subsequently fails
        let sent = matches!(presented, Presented::Sent(_));
        if sent { self.presented += 1; } else { self.reused += 1; }
        let initial = self.display.as_ref().ok_or_else(|| io::Error::other("benchmark startup was not reported"))?;
        if display.geometry != initial.geometry || display.renderer != initial.renderer || display.graphics_protocol != initial.graphics_protocol {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark display geometry or renderer changed"));
        }
        self.write_record(&Frame { r#type: "frame", index: self.completed, simulation_time_ms: time_ms,
            elapsed_seconds: elapsed, presentation: if sent { "presented" } else { "reused_display" },
            display, trace_complete: true, steps: times.capture_steps()? })?;
        times.clear_capture_segment()?;
        Ok(self.completed == self.settings.frames)
    }

    fn write_end(&mut self, result: &io::Result<()>) -> io::Result<()> {
        let status = match result { Ok(()) => "complete", Err(error) if error.kind() == io::ErrorKind::Interrupted => "aborted", Err(_) => "failed" };
        self.write_record(&serde_json::json!({"type": "end", "status": status, "requested_frames": self.settings.frames,
            "completed_frames": self.completed, "presented_frames": self.presented, "reused_display_frames": self.reused}))
    }
}

pub(crate) fn open_frame_benchmark(config: &Config) -> Result<Option<BenchmarkSession>, ExitCode> {
    let Some(settings) = &config.frame_benchmark else { return Ok(None); };
    let file = File::options().write(true).create_new(true).open(&settings.report)
        .map_err(|error| super::report_failure(format!("Cannot create benchmark report {}: {error}", settings.report.display())))?;
    Ok(Some(BenchmarkSession::new(BufWriter::new(file), settings.clone())))
}

/// Write the startup report before the caller starts simulated time.
pub(crate) fn begin_benchmark_capture(session: &mut Option<BenchmarkSession>, config: &Config, display: impl FnOnce() -> io::Result<CaptureDisplay>, times: &mut StepTimes) -> io::Result<()> {
    // Finish startup timing before writing the report or starting the simulation clock.
    times.measure("END");

    // Capture terminal details only when a benchmark report was requested.
    if let Some(session) = session {
        session.begin_capture(config, display()?, times)?;
    }
    Ok(())
}

/// Reject controls and resizes during a benchmark, before either can change the workload.
pub(crate) fn validate_benchmark_input(session: &Option<BenchmarkSession>, input: &FrameInput) -> io::Result<()> {
    // Interactive runs accept input; benchmarks require unchanged controls and terminal dimensions.
    if session.is_none() { return Ok(()); }
    if input.resized || !input.controls.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark interrupted by terminal input or resize"));
    }
    Ok(())
}

/// Set a benchmark's next frame date without changing the clock during interactive runs.
pub(crate) fn set_benchmark_frame_time(session: &Option<BenchmarkSession>, clock: &mut SimulationClock) -> io::Result<()> {
    // Completed frames determine the next zero-based frame index and its scheduled simulation date.
    let Some(session) = session else { return Ok(()); };
    clock.set_frame_index(session.completed)
}

pub(crate) fn complete_benchmark_frame(
    session: &mut Option<BenchmarkSession>,
    time_ms: i64,
    presented: Presented,
    elapsed: f64,
    display: &Display,
    times: &mut StepTimes,
) -> io::Result<bool> {
    // Ordinary runs have no benchmark report to update.
    let Some(session) = session else {
        return Ok(false);
    };

    // Read the display information and record the completed frame.
    let display_info = capture_display(display.info())?;
    session.record_frame(time_ms, presented, elapsed, display_info, times)
}

pub(crate) fn finish_benchmark_report(session: &mut Option<BenchmarkSession>, mut result: io::Result<()>) -> io::Result<()> {
    let Some(session) = session else { return result; };
    if result.is_ok() && session.completed != session.settings.frames { result = Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark ended before the requested frame count")); }
    if let Err(error) = session.write_end(&result) {
        if result.is_ok() { return Err(error); }
        eprintln!("Additional benchmark report error: {error}");
    }
    result
}

#[cfg(test)]
mod tests;
