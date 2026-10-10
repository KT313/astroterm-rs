use super::*;
#[test]
fn disabled_and_plain_targets_preserve_partial_notices() {
    for updates in [false, true] {
        let mut reporter = StartupProgress::new(Vec::new(), false, updates);
        reporter.write_all(b"warn").unwrap(); reporter.write_all(b"ing\nnext").unwrap();
        reporter.flush().unwrap();
        assert_eq!(reporter.writer, b"warning\nnext");
        reporter.begin_stage(Stage::Parse, "", Unit::Rows, None);
        reporter.set_position(4096); reporter.finish(false);
        assert!(!reporter.writer.contains(&27));
        assert!(!reporter.writer.contains(&b'\r'));
        assert!(reporter.display.is_none());
    }
}

#[test]
fn hidden_animation_target_does_not_swallow_notices_and_buffer_is_bounded() {
    let mut reporter = StartupProgress::new(Vec::new(), false, false);
    reporter.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::hidden()));
    let text = format!("{}\nlast fragment", "界".repeat(5000));
    for bytes in text.as_bytes().chunks(7) {
        reporter.write_all(bytes).unwrap();
        assert!(reporter.pending.len() < STARTUP_NOTICE_BYTES);
    }
    reporter.finish(true);
    assert_eq!(reporter.writer, text.as_bytes());
    assert!(reporter.task.is_none() && reporter.files.is_none() && reporter.display.is_none());
}

#[test]
fn thousands_of_file_updates_keep_only_two_bars() {
    let mut reporter = StartupProgress::new(Vec::new(), false, false);
    reporter.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::hidden()));
    for file in 0..3386 {
        reporter.set_files(file, 3386);
        reporter.begin_stage(Stage::Verify, "file", Unit::Bytes, Some(128));
        reporter.set_position(128);
        reporter.finish_stage(Outcome::Complete);
    }
    assert!(reporter.task.is_some() && reporter.files.is_some());
    reporter.finish(true);
}

#[test]
fn warning_errors_propagate_and_panic_cleanup_does_not_panic_again() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::Error::other("writer failed")) }
        fn flush(&mut self) -> io::Result<()> { Err(io::Error::other("flush failed")) }
    }
    let mut reporter = StartupProgress::new(Broken, false, false);
    assert!(reporter.write_all(b"warning\n").is_err());
    reporter.finish(false);
    let result = std::panic::catch_unwind(|| {
        let mut reporter = StartupProgress::new(Broken, false, false);
        reporter.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::hidden()));
        reporter.begin_stage(Stage::Parse, "", Unit::Rows, None);
        panic!("original failure");
    });
    assert!(result.is_err());
}
