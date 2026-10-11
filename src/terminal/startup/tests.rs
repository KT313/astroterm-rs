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
fn plain_writer_announces_stage_changes_and_counts_at_most_once_per_second() {
    let mut reporter = StartupProgress::new(Vec::new(), false, true);
    reporter.begin_stage(Stage::Download, "GaiaSource_000000-003111.csv.gz", Unit::Bytes, Some(230_000_000));
    reporter.set_position(10_000_000);                                                       // within a second of the announcement
    for _ in 0..3 { reporter.begin_stage(Stage::Verify, "file", Unit::Bytes, Some(128)); reporter.finish_stage(Outcome::Complete); }
    reporter.begin_stage(Stage::Parse, "", Unit::Rows, None);
    reporter.plain.as_mut().unwrap().last_line -= Duration::from_secs(2);
    reporter.set_position(4096);
    reporter.set_position(8192);                                                             // within a second of the last count line
    reporter.begin_stage(Stage::Download, "", Unit::Bytes, Some(230_000_000));
    reporter.plain.as_mut().unwrap().last_line -= Duration::from_secs(2);
    reporter.set_position(100_000_000);
    reporter.begin_stage(Stage::Sort, "", Unit::Work, None);
    reporter.plain.as_mut().unwrap().last_line -= Duration::from_secs(2);
    reporter.set_position(7);                                                                // work stages have no count line
    reporter.finish(true);
    assert_eq!(String::from_utf8_lossy(&reporter.writer),
        "Downloading...\nVerifying source...\nLoading stars...\nLoading stars: 4096 rows\nDownloading...\nDownloading: 100 MB of 230 MB\nSorting stars by region...\n");
    let mut silent = StartupProgress::new(Vec::new(), false, false);
    silent.begin_stage(Stage::Parse, "", Unit::Rows, None);
    silent.set_position(4096);
    assert!(silent.writer.is_empty());
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

#[test]
fn block_progress_uses_one_bar_and_plain_counts_name_blocks() {
    let mut reporter = StartupProgress::new(Vec::new(), false, true);
    reporter.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(10));
    reporter.plain.as_mut().unwrap().last_line -= Duration::from_secs(2);
    reporter.set_position(3);
    assert_eq!(String::from_utf8_lossy(&reporter.writer), "Downloading...\nDownloading: 3 of 10 blocks\n");
    let mut visible = StartupProgress::new(Vec::new(), false, false);
    visible.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::hidden()));
    visible.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(10));
    for ready in 0..=10 { visible.set_position(ready); }
    assert_eq!(visible.task.as_ref().unwrap().position(), 10);
    assert_eq!(visible.task.as_ref().unwrap().length(), Some(10));
    assert!(visible.files.is_none());
}

#[test]
fn unified_block_bar_displays_aggregate_speed_without_changing_position() {
    let mut reporter = StartupProgress::new(Vec::new(), false, true);
    reporter.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(10));
    reporter.set_position(3);
    reporter.plain.as_mut().unwrap().last_line -= Duration::from_secs(2);
    reporter.set_download_bytes(8 * 1024 * 1024, Duration::from_secs(2));
    assert_eq!(String::from_utf8_lossy(&reporter.writer), "Downloading...\nDownloading: 3 of 10 blocks; 4.00 MiB/s avg\n");
    reporter.begin_stage(Stage::Parse, "", Unit::Rows, Some(100));
    assert!(reporter.download_rate.is_none());

    let mut visible = StartupProgress::new(Vec::new(), false, false);
    visible.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::hidden()));
    visible.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(10));
    visible.set_position(3);
    visible.set_download_bytes(8 * 1024 * 1024, Duration::from_secs(2));
    assert_eq!(visible.task.as_ref().unwrap().position(), 3);
    assert_eq!(visible.task.as_ref().unwrap().message(), "4.00 MiB/s avg");
    visible.set_download_bytes(0, Duration::ZERO);
    assert_eq!(visible.task.as_ref().unwrap().message(), "0.00 MiB/s avg");

    let mut silent = StartupProgress::new(Vec::new(), false, false);
    silent.begin_stage(Stage::Download, "", Unit::Blocks, Some(1));
    silent.set_download_bytes(1024, Duration::from_secs(1));
    assert!(silent.writer.is_empty());
}

#[test]
fn block_eta_ignores_cached_work_and_waits_for_a_new_completion() {
    assert_eq!(estimate_block_eta(Some(500), 250, 250, Duration::from_secs(20)), None);
    assert_eq!(estimate_block_eta(Some(500), 250, 255, Duration::from_secs(20)), Some(Duration::from_secs(980)));
    assert_eq!(estimate_block_eta(Some(250), 0, 5, Duration::from_secs(20)), Some(Duration::from_secs(980))); // same remaining work and new completions, without the cached prefix
    assert_eq!(estimate_block_eta(Some(500), 250, 255, Duration::from_secs(40)), Some(Duration::from_secs(1960))); // a stall lengthens the estimate
    assert_eq!(estimate_block_eta(Some(500), 250, 500, Duration::from_secs(20)), Some(Duration::ZERO));
    assert_eq!(estimate_block_eta(Some(500), 250, 249, Duration::from_secs(20)), None);
    assert_eq!(estimate_block_eta(Some(500), 250, 251, Duration::ZERO), None);
    assert_eq!(estimate_block_eta(None, 250, 255, Duration::from_secs(20)), None);
    assert_eq!(estimate_block_eta(Some(u64::MAX), 0, 1, Duration::MAX), None);
}

#[derive(Clone, Debug, Default)]
struct CapturedTerminal(std::sync::Arc<std::sync::Mutex<String>>);
impl indicatif::TermLike for CapturedTerminal {
    fn width(&self) -> u16 { 180 }
    fn move_cursor_up(&self, _: usize) -> io::Result<()> { Ok(()) }
    fn move_cursor_down(&self, _: usize) -> io::Result<()> { Ok(()) }
    fn move_cursor_right(&self, _: usize) -> io::Result<()> { Ok(()) }
    fn move_cursor_left(&self, _: usize) -> io::Result<()> { Ok(()) }
    fn write_line(&self, text: &str) -> io::Result<()> { self.write_str(text) }
    fn write_str(&self, text: &str) -> io::Result<()> { self.0.lock().unwrap().push_str(text); Ok(()) }
    fn clear_line(&self) -> io::Result<()> { Ok(()) }
    fn flush(&self) -> io::Result<()> { Ok(()) }
}
fn draw_progress(reporter: &StartupProgress<Vec<u8>>, terminal: &CapturedTerminal) -> String {
    let bar = reporter.task.as_ref().unwrap();
    bar.disable_steady_tick();
    terminal.0.lock().unwrap().clear();
    bar.force_draw();
    terminal.0.lock().unwrap().clone()
}

#[test]
fn cached_block_baseline_preserves_the_visible_count_and_resets_between_stages() {
    let terminal = CapturedTerminal::default();
    let mut visible = StartupProgress::new(Vec::new(), false, false);
    visible.display = Some(MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(terminal.clone()))));
    visible.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(500));
    visible.set_cached_blocks(250);
    let initial = draw_progress(&visible, &terminal);
    assert!(initial.contains("250/500 blocks") && initial.contains("ETA --"), "{initial}");
    visible.set_position(255);
    visible.set_download_bytes(8 * 1024 * 1024, Duration::from_secs(2));
    let updated = draw_progress(&visible, &terminal);
    assert!(updated.contains("255/500 blocks") && updated.contains("4.00 MiB/s avg"), "{updated}");
    assert!(!updated.contains("ETA --"), "{updated}");
    visible.begin_stage(Stage::Download, "next selection", Unit::Blocks, Some(10));
    visible.set_cached_blocks(0);
    let reset = draw_progress(&visible, &terminal);
    assert!(reset.contains("0/10 blocks") && reset.contains("ETA --"), "{reset}");
    visible.set_position(1);
    assert!(!draw_progress(&visible, &terminal).contains("ETA --"));                              // the previous selection's cached baseline must not leak
}
