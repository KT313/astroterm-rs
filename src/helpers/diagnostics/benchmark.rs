//! One report writer per process; timings stay in StepTimes and are released after each serialized segment.
use astroterm::{astro::{FrameTime, SimulationClock}, model::{Config, FrameBenchmarkConfig}, present::Presented,
    terminal::FrameInput, timing::{CaptureSteps, StepTimes}};
use super::CaptureDisplay;
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
}

#[derive(Serialize)]
struct Segment<'a> { r#type: &'static str, trace_complete: bool, steps: CaptureSteps<'a> }
#[derive(Serialize)]
struct Frame<'a> {
    r#type: &'static str, index: u32, utc_jd: f64, tt_jd: f64, elapsed_seconds: f64,
    presentation: &'static str, display: CaptureDisplay, trace_complete: bool, steps: CaptureSteps<'a>,
}

impl<W: Write> BenchmarkSession<W> {
    fn new(writer: W, settings: FrameBenchmarkConfig) -> Self {
        Self { writer, settings, completed: 0, presented: 0, reused: 0, display: None, write_failed: false }
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
            "type": "run", "schema_version": 1, "requested_frames": self.settings.frames,
            "step_seconds": self.settings.step_seconds, "start_utc_jd": config.simulation.start_julian_date,
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
        Ok(())
    }

    fn record_frame(&mut self, time: FrameTime, presented: Presented, elapsed: f64, display: CaptureDisplay, times: &mut StepTimes) -> io::Result<bool> {
        if self.completed >= self.settings.frames || !elapsed.is_finite() || elapsed < 0.0 || ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) { return Err(io::Error::other("invalid benchmark frame completion")); }
        self.completed += 1;                                                                  // successful rendering counts even when reporting subsequently fails
        let sent = matches!(presented, Presented::Sent(_));
        if sent { self.presented += 1; } else { self.reused += 1; }
        let initial = self.display.as_ref().ok_or_else(|| io::Error::other("benchmark startup was not reported"))?;
        if display.geometry != initial.geometry || display.renderer != initial.renderer || display.graphics_protocol != initial.graphics_protocol {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark display geometry or renderer changed"));
        }
        self.write_record(&Frame { r#type: "frame", index: self.completed, utc_jd: time.utc, tt_jd: time.tt,
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
    // Capture terminal details only when a benchmark report was requested.
    if let Some(session) = session {
        session.begin_capture(config, display()?, times)?;
    }
    Ok(())
}

pub(crate) fn prepare_benchmark_frame(session: &Option<BenchmarkSession>, input: &FrameInput, clock: &mut SimulationClock) -> io::Result<()> {
    if let Some(session) = session {
        if input.resized || !input.controls.is_empty() { return Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark interrupted by terminal input or resize")); }
        clock.set_frame_index(session.completed)?;
    }
    Ok(())
}

pub(crate) fn validate_benchmark_time(session: &Option<BenchmarkSession>, time: FrameTime) -> io::Result<()> {
    if session.is_some() && ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "benchmark frame time is not finite"));
    }
    Ok(())
}

pub(crate) fn complete_benchmark_frame(session: &mut Option<BenchmarkSession>, time: FrameTime, presented: Presented, elapsed: f64, display: impl FnOnce() -> io::Result<CaptureDisplay>, times: &mut StepTimes) -> io::Result<bool> {
    match session { Some(session) => session.record_frame(time, presented, elapsed, display()?, times), None => Ok(false) }
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
