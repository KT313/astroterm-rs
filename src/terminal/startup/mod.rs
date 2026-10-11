//! Scoped startup display; it stops before raw mode, terminal probes or the alternate screen open.
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, ProgressState, HumanDuration};
use crate::progress::{DatasetProgress, Stage, Unit, Outcome};
use crate::constants::{STARTUP_PROGRESS_HZ, STARTUP_NOTICE_BYTES};
use std::{io::{self, IsTerminal, Write}, time::{Duration, Instant}};

pub struct StartupProgress<W: Write> {
    writer: W,
    display: Option<MultiProgress>,
    task: Option<ProgressBar>,
    files: Option<ProgressBar>,
    pending: Vec<u8>,
    plain_updates: bool,
    plain: Option<PlainStage>,
    download_rate: Option<String>,
    active: Option<Stage>,
    finished: bool,
}

/// The stage last announced on a plain writer; count lines repeat its label at most once per second.
struct PlainStage { stage: Stage, unit: Unit, total: Option<u64>, last_line: Instant, position: u64 }

pub fn open_startup_progress(disabled: bool) -> StartupProgress<io::Stderr> {
    let writer = io::stderr(); // never hold stderr.lock() while the animation thread draws
    let visible = !disabled && writer.is_terminal() && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
    StartupProgress::new(writer, visible, !disabled)
}

impl<W: Write> StartupProgress<W> {
    fn new(writer: W, visible: bool, plain_updates: bool) -> Self {
        let display = visible.then(|| MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(STARTUP_PROGRESS_HZ)));
        Self { writer, display, task: None, files: None, pending: Vec::new(), plain_updates, plain: None, download_rate: None, active: None, finished: false }
    }
    pub fn finish(&mut self, success: bool) {
        if self.finished { return; }
        self.finish_stage(if success { Outcome::Complete } else { Outcome::Failed });
        self.stop_display();
        let _ = self.flush();
        self.finished = true;
    }
    fn stop_display(&mut self) {
        if let Some(task) = self.task.take() { task.disable_steady_tick(); task.finish_and_clear(); }
        if let Some(files) = self.files.take() { files.finish_and_clear(); }
        if let Some(display) = self.display.take() { let _ = display.clear(); }
    }
    fn emit_pending(&mut self) -> io::Result<()> {
        if self.pending.is_empty() { return Ok(()); }
        self.emit_prefix(self.pending.len())
    }
    fn emit_prefix(&mut self, length: usize) -> io::Result<()> {
        if self.pending[..length].last() != Some(&b'\n') { self.stop_display(); } // don't redraw over an unfinished notice line
        let result = if let Some(display) = &self.display {
            display.suspend(|| self.writer.write_all(&self.pending[..length]))
        } else { self.writer.write_all(&self.pending[..length]) };
        if result.is_ok() { self.pending.drain(..length); }
        result
    }
}
impl<W: Write> Write for StartupProgress<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.display.is_none() { return self.writer.write(bytes); }
        for (index, &byte) in bytes.iter().enumerate() {
            self.pending.push(byte);
            if byte == b'\n' { self.emit_pending()?; }
            else if self.pending.len() >= STARTUP_NOTICE_BYTES {
                let length = match std::str::from_utf8(&self.pending) {
                    Err(error) if error.error_len().is_none() => error.valid_up_to(), // don't insert redraw sequences inside a split UTF-8 character
                    _ => self.pending.len(),
                };
                self.emit_prefix(length)?;
            }
            if self.display.is_none() {
                self.emit_pending()?;
                self.writer.write_all(&bytes[index + 1..])?;
                return Ok(bytes.len());
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { self.emit_pending()?; self.writer.flush() }
}
impl<W: Write> DatasetProgress for StartupProgress<W> {
    fn begin_stage(&mut self, stage: Stage, detail: &str, unit: Unit, total: Option<u64>) {
        self.active = Some(stage);
        self.download_rate = None;
        let Some(display) = &self.display else {
            if self.plain_updates {
                let recent = self.plain.as_ref().filter(|plain| plain.stage == stage && plain.last_line.elapsed() < Duration::from_secs(1));
                let last_line = match recent {
                    Some(plain) => plain.last_line, // thousands of quick file checks print one line per second, not one each
                    None => { let _ = writeln!(self.writer, "{}{}...", stage.label(), if detail.is_empty() { String::new() } else { format!(" ({detail})") }); Instant::now() }
                };
                self.plain = Some(PlainStage { stage, unit, total, last_line, position: 0 });
            }
            return;
        };
        let bar = self.task.get_or_insert_with(|| display.add(ProgressBar::new_spinner()));
        bar.disable_steady_tick(); bar.reset();
        let total = total.filter(|&n| n != 0);
        if let Some(total) = total { bar.set_length(total); } else { bar.unset_length(); }
        if let Some(style) = build_work_style(unit, total) { bar.set_style(style); }
        bar.set_prefix(if detail.is_empty() { stage.label().to_owned() } else { format!("{} ({detail})", stage.label()) }); bar.set_message("");
        bar.enable_steady_tick(Duration::from_millis(1000 / u64::from(STARTUP_PROGRESS_HZ.max(1))));
    }
    fn set_position(&mut self, completed: u64) {
        if let Some(bar) = &self.task { bar.set_position(completed); return; }
        let Some(plain) = &mut self.plain else { return; };
        plain.position = completed;
        if self.display.is_some() || plain.last_line.elapsed() < Duration::from_secs(1) { return; }
        let count = match (plain.unit, plain.total) {
            (Unit::Work, _) => return,
            (Unit::Bytes, Some(total)) => format!("{} MB of {} MB", completed / 1_000_000, total / 1_000_000),
            (Unit::Bytes, None) => format!("{} MB", completed / 1_000_000),
            (Unit::Rows, total) => describe_count(completed, total, "rows"),
            (Unit::Stars, total) => describe_count(completed, total, "stars"),
            (Unit::Elements, total) => describe_count(completed, total, "field values"),
            (Unit::Blocks, total) => describe_count(completed, total, "blocks"),
        };
        let rate = self.download_rate.as_ref().map(|rate| format!("; {rate}")).unwrap_or_default();
        let _ = writeln!(self.writer, "{}: {count}{rate}", plain.stage.label());
        plain.last_line = Instant::now();
    }
    fn set_detail(&mut self, detail: &str) {
        if let Some(bar) = &self.task { bar.set_message(detail.to_owned()); return; }
        let Some(plain) = &mut self.plain else { return; };
        if self.plain_updates && plain.last_line.elapsed() >= Duration::from_secs(1) {
            let _ = writeln!(self.writer, "{}: {detail}", plain.stage.label());
            plain.last_line = Instant::now();
        }
    }
    fn set_work(&mut self, unit: Unit, total: Option<u64>) {
        if let Some(plain) = &mut self.plain { plain.unit = unit; plain.total = total; }
        if let Some(bar) = &self.task {
            if let Some(total) = total { bar.set_length(total); } else { bar.unset_length(); }
            if let Some(style) = build_work_style(unit, total) { bar.set_style(style); }
        }
    }
    fn set_read_progress(&mut self, bytes: u64, total: u64) {
        if self.task.is_none() && !self.plain_updates { return; }
        self.set_detail(&format!("Reading cached data: {:.1}/{:.1} MiB", bytes as f64 / 1_048_576.0, total as f64 / 1_048_576.0));
    }
    fn set_cached_blocks(&mut self, ready: u64) {
        if let Some(bar) = &self.task { bar.set_style(with_block_eta(bar.style(), ready)); }
        self.set_position(ready);                                                              // keep the full ready count, but exclude its initial value from ETA
    }
    fn set_download_bytes(&mut self, received: u64, elapsed: Duration) {
        if self.active.is_none() || (self.task.is_none() && !self.plain_updates) { return; }
        let seconds = elapsed.as_secs_f64();
        let rate = if seconds > 0.0 { received as f64 / seconds / 1_048_576.0 } else { 0.0 };
        if self.active != Some(Stage::Download) {
            self.set_detail(&format!("Repair download: {:.1} MiB received; {rate:.2} MiB/s avg", received as f64 / 1_048_576.0));
            return;                                                                           // keep the loading count and elapsed time while showing repair traffic
        }
        let message = format!("{rate:.2} MiB/s avg");
        if let Some(bar) = &self.task { bar.set_message(message); return; }
        self.download_rate = Some(message);
        if let Some(plain) = &self.plain { self.set_position(plain.position); }                   // preserve the block count while refreshing plain-output speed
    }
    fn set_retained(&mut self, retained: u64) {
        if let Some(bar) = &self.task { bar.set_message(format!("{retained} sources retained")); }
    }
    fn set_files(&mut self, ready: u64, total: u64) {
        if let Some(display) = &self.display {
            if total <= 1 { return; }
            let bar = self.files.get_or_insert_with(|| {
                let bar = display.insert(0, ProgressBar::new(total));
                if let Ok(style) = ProgressStyle::with_template("Source files ready {wide_bar} {pos}/{len}") { bar.set_style(style); }
                bar
            });
            bar.set_length(total); bar.set_position(ready);
        }
    }
    fn finish_stage(&mut self, outcome: Outcome) {
        self.active = None;
        if let Some(bar) = &self.task {
            bar.disable_steady_tick();
            match outcome { Outcome::Complete => bar.finish_and_clear(), Outcome::Failed => bar.abandon_with_message("failed") }
        }
    }
}
fn build_work_style(unit: Unit, total: Option<u64>) -> Option<ProgressStyle> {
    let template = match (unit, total.is_some()) {
        (Unit::Bytes, true) => "{spinner} {prefix} {wide_bar} {bytes}/{total_bytes} {bytes_per_sec} ETA {eta} {msg}",
        (Unit::Bytes, false) => "{spinner} {prefix} {bytes} {bytes_per_sec} {elapsed} {msg}",
        (Unit::Blocks, true) => "{spinner} {prefix} {wide_bar}\n{pos}/{len} blocks ETA {block_eta} {wide_msg}", // changing activity text cannot resize the bar
        (Unit::Work, _) => "{spinner} {prefix} {elapsed} {msg}",
        (_, true) => "{spinner} {prefix} {wide_bar} {pos}/{len} {elapsed} {msg}",
        (_, false) => "{spinner} {prefix} {pos} rows {elapsed} {msg}",
    };
    ProgressStyle::with_template(template).ok().map(|style| if unit == Unit::Blocks { with_block_eta(style, 0) } else { style })
}

fn with_block_eta(style: ProgressStyle, cached: u64) -> ProgressStyle {
    style.with_key("block_eta", move |state: &ProgressState, output: &mut dyn std::fmt::Write| {
        match estimate_block_eta(state.len(), cached, state.pos(), state.elapsed()) {
            Some(eta) => { let _ = write!(output, "{:#}", HumanDuration(eta)); },
            None => { let _ = output.write_str("--"); },                                        // wait for a new completed block before estimating speed
        }
    })
}

fn estimate_block_eta(total: Option<u64>, cached: u64, ready: u64, elapsed: Duration) -> Option<Duration> {
    let remaining = total?.saturating_sub(ready);
    if remaining == 0 { return Some(Duration::ZERO); }
    let completed_now = ready.saturating_sub(cached);
    if completed_now == 0 || elapsed.is_zero() { return None; }
    Duration::try_from_secs_f64(elapsed.as_secs_f64() * remaining as f64 / completed_now as f64).ok()
}

fn describe_count(completed: u64, total: Option<u64>, unit: &str) -> String {
    match total { Some(total) => format!("{completed} of {total} {unit}"), None => format!("{completed} {unit}") }
}
impl<W: Write> Drop for StartupProgress<W> {
    fn drop(&mut self) { self.stop_display(); let _ = self.flush(); } // cleanup never claims that unfinished work succeeded
}

#[cfg(test)]
mod tests;
