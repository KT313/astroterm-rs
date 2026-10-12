use super::*;
use astroterm::{cli::{Arguments, build_config}, terminal::CaptureGeometry};
use clap::Parser;

fn config() -> Config {
    build_config(Arguments::try_parse_from(["astroterm", "--debug-frames", "3", "--debug-frame-report", "unused.jsonl", "-d", "2025-03-01T11:00:00"]).unwrap(), &[]).unwrap()
}
fn display() -> CaptureDisplay {
    CaptureDisplay { renderer: "pixels", graphics_protocol: "kitty", transport: "streaming", compression_support: "unknown", tmux: false,
        geometry: CaptureGeometry { screen_cells: [80, 24], font_pixels: Some([8, 16]), image_pixels: Some([640, 384]) } }
}

#[test]
fn completed_frames_include_reuse_and_keep_startup_separate() {
    let config = config();
    let mut session = BenchmarkSession::new(Vec::new(), config.frame_benchmark.clone().unwrap());
    let mut times = StepTimes::with_trace(true);
    times.measure("Startup fixture", || {});
    session.begin_capture(&config, display(), &mut times).unwrap();
    for index in 1..=3 {
        times.begin_frame();
        times.measure_steps("Observation", |times| { times.measure("Repeated", || {}); times.measure("Repeated", || {}); });
        let outcome = if index == 1 { Presented::Sent(1) } else { Presented::Unchanged };
        let done = session.record_frame(FrameTime::from_utc(config.simulation.start_julian_date), outcome, 0.000_123_456, display(), &mut times).unwrap();
        assert_eq!(done, index == 3);
        assert!(times.trace().unwrap().steps.is_empty());
        assert!(!times.steps().is_empty());                                                     // segment disposal must not reset the display averages
    }
    session.write_end(&Ok(())).unwrap();
    let records: Vec<serde_json::Value> = String::from_utf8(session.writer).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(records.len(), 6);
    assert_eq!(records[1]["steps"][0]["name"], "Startup fixture");
    assert_eq!(records[2]["elapsed_seconds"], 0.000_123_456);
    assert_eq!(records[2]["steps"][1]["parent"], 0);
    assert_eq!(records[3]["presentation"], "reused_display");
    assert_eq!(records[5]["completed_frames"], 3);
    assert_eq!(records[5]["presented_frames"], 1);
    assert_eq!(records[5]["reused_display_frames"], 2);
}

#[test]
fn failed_report_flush_and_geometry_change_cannot_complete_capture() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Err(io::Error::other("injected flush error")) }
    }
    let config = config();
    let mut broken = BenchmarkSession::new(Broken, config.frame_benchmark.clone().unwrap());
    assert!(broken.write_record(&serde_json::json!({"type":"run"})).is_err());
    assert!(broken.write_end(&Ok(())).is_err());

    let mut session = BenchmarkSession::new(Vec::new(), config.frame_benchmark.clone().unwrap());
    let mut times = StepTimes::with_trace(true);
    session.begin_capture(&config, display(), &mut times).unwrap();
    let mut changed = display();
    changed.geometry.screen_cells[0] += 1;
    let error = session.record_frame(FrameTime::from_utc(2451545.0), Presented::Sent(1), 0.1, changed, &mut times).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    session.write_end(&Err(error)).unwrap();
    let records: Vec<serde_json::Value> = String::from_utf8(session.writer).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(records.last().unwrap()["status"], "aborted");
    assert!(!records.iter().any(|record| record["type"] == "frame"));
}

#[test]
fn terminal_controls_abort_and_existing_report_files_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    let path = root.path().join("capture.jsonl");
    config.frame_benchmark.as_mut().unwrap().report = path.clone();
    let mut session = open_frame_benchmark(&config).unwrap();
    let mut clock = SimulationClock::start_fixed_step(2451545.0, 0.0);
    let input = FrameInput { controls: Vec::new(), resized: true };
    assert!(prepare_benchmark_frame(&session, &input, &mut clock).is_err());
    assert!(finish_benchmark_report(&mut session, Ok(())).is_err());
    let bytes = std::fs::read(&path).unwrap();
    assert!(open_frame_benchmark(&config).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn report_failure_keeps_the_operational_error_primary() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    config.frame_benchmark.as_mut().unwrap().report = root.path().join("report.jsonl");
    let mut session = open_frame_benchmark(&config).unwrap();
    session.as_mut().unwrap().write_failed = true;
    let error = finish_benchmark_report(&mut session, Err(io::Error::other("original rendering failure"))).unwrap_err();
    assert_eq!(error.to_string(), "original rendering failure");
}
