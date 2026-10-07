use super::*;
use clap::Parser;
use astroterm::cli::build_config;
#[cfg(feature = "memory-diagnostics")]
use astroterm::terminal::FrameInput;
#[cfg(feature = "memory-diagnostics")]
use crate::helpers::stop_on_quit;

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

#[test]
fn disabled_table_logging_does_not_access_the_requested_path() {
    let config = report_config(&["astroterm"]);
    let state = ApplicationState::new(config, StepTimes::default());
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, "unchanged").unwrap();
    log_pipeline_data_if_requested(&state, blocker.join("tables.md"), "disabled").unwrap();
    log_pipeline_data_if_requested(&state, directory.path().join("missing/tables.md"), "disabled").unwrap();
    assert_eq!(std::fs::read_to_string(&blocker).unwrap(), "unchanged");
    assert!(!directory.path().join("missing").exists());
}

#[test]
fn requested_table_logs_create_parents_append_and_propagate_io_errors() {
    let config = report_config(&["astroterm", "--debug-log-data"]);
    let state = ApplicationState::new(config, StepTimes::default());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested/tables.md");
    log_pipeline_data_if_requested(&state, &path, "first").unwrap();
    log_pipeline_data_if_requested(&state, &path, "second").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.find("## first").unwrap() < text.find("## second").unwrap());
    assert!(log_pipeline_data_if_requested(&state, path.join("cannot-create.md"), "failed").is_err());
}
