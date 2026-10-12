use super::*;
use clap::Parser;
use astroterm::cli::build_config;

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
