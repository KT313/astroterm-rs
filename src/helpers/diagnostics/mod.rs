//! Optional trace descriptions and reporting after terminal cleanup.
mod benchmark;
pub(crate) use benchmark::{BenchmarkSession, open_frame_benchmark, begin_benchmark_capture, prepare_benchmark_frame,
    validate_benchmark_time, complete_benchmark_frame, finish_benchmark_report};
use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::time::Instant;
use astroterm::astro::FrameTime;
use astroterm::model::Config;
use astroterm::present::{CompressionSupport, DisplayInfo, Presented};
use serde::Serialize;
use astroterm::time::StepTimes;
use ratatui_image::picker::ProtocolType;

/// Report diagnostics and the original rendering result after the terminal scope has dropped its guard.
pub(crate) fn finish_rendering(result: io::Result<()>, config: &Config, times: &StepTimes) -> ExitCode {
    #[cfg(feature = "pixel-debug")]
    if let Err(error) = astroterm::pixel_debug::write_report(&mut io::stderr().lock()) { eprintln!("Pixel diagnostic report failed: {error}"); }
    finish_requested_report(result, config, times, &mut io::stdout().lock(), &mut io::stderr().lock())
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

/// Record the duration of a successful frame through presentation, whether or not anything was sent.
pub(crate) fn finish_frame_diagnostics(config: &Config, frame_start: Instant, time: FrameTime, presented: Presented, times: &mut StepTimes) -> f64 {
    times.record_frame_presentation(matches!(presented, Presented::Sent(_)));
    if !(config.debug_singleframe || config.frame_benchmark.is_some()) { return 0.0; }
    let elapsed = frame_start.elapsed().as_secs_f64();
    times.describe("Present", || format!(
        "{}; frame elapsed through presentation={:.3} ms; UTC JD={:.9}; includes in-frame diagnostics; sleep excluded",
        match presented { Presented::Sent(bytes) => format!("sent {bytes} bytes"), Presented::Unchanged => "unchanged frame, nothing sent".into() },
        elapsed * 1000.0, time.utc
    ));
    elapsed
}

/// The terminal geometry a benchmark record carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CaptureGeometry {
    pub screen_cells: [u32; 2],
    pub font_pixels: Option<[u32; 2]>,
    pub image_pixels: Option<[u32; 2]>,
}

/// The display facts a benchmark record carries: small owned values, no image copies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CaptureDisplay {
    pub renderer: &'static str,
    pub graphics_protocol: &'static str,
    pub geometry: CaptureGeometry,
    pub transport: &'static str,
    pub compression_support: &'static str,
    pub tmux: bool,
}

/// The display facts a benchmark record carries, checked against the terminal's current size.
pub(crate) fn capture_display(info: DisplayInfo) -> io::Result<CaptureDisplay> {
    let protocol = match info.protocol { ProtocolType::Kitty => "kitty", ProtocolType::Sixel => "sixel", ProtocolType::Iterm2 => "iterm2", ProtocolType::Halfblocks => "halfblocks" };
    let compression_support = match info.compression { CompressionSupport::Supported => "supported", CompressionSupport::Unsupported => "unsupported", CompressionSupport::Unknown => "unknown" };
    let display = CaptureDisplay { renderer: "pixels", graphics_protocol: protocol, compression_support, tmux: info.tmux, transport: if info.shared_memory { "shared_memory" } else { "streaming" },
        geometry: CaptureGeometry { screen_cells: [u32::from(info.screen.width), u32::from(info.screen.height)],
            font_pixels: Some([u32::from(info.font.0), u32::from(info.font.1)]), image_pixels: Some([info.image.0 as u32, info.image.1 as u32]) } };
    let (columns, rows) = crossterm::terminal::size()?;
    if display.geometry.screen_cells != [u32::from(columns), u32::from(rows)] {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "benchmark terminal resized during frame processing"));
    }
    Ok(display)
}

#[cfg(test)]
mod tests;
