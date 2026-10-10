//! Scoped startup display; it stops before raw mode, terminal probes or the alternate screen open.
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
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
    last_plain: Option<Instant>,
    active: Option<Stage>,
    finished: bool,
}

pub fn open_startup_progress(disabled: bool) -> StartupProgress<io::Stderr> {
    let writer = io::stderr(); // never hold stderr.lock() while the animation thread draws
    let visible = !disabled && writer.is_terminal() && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
    StartupProgress::new(writer, visible, !disabled)
}

impl<W: Write> StartupProgress<W> {
    fn new(writer: W, visible: bool, plain_updates: bool) -> Self {
        let display = visible.then(|| MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(STARTUP_PROGRESS_HZ)));
        Self { writer, display, task: None, files: None, pending: Vec::new(), plain_updates, last_plain: None, active: None, finished: false }
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
        let Some(display) = &self.display else {
            if self.plain_updates && self.last_plain.is_none_or(|last| last.elapsed() >= Duration::from_secs(1)) {
                let _ = writeln!(self.writer, "{}...", stage.label());
                self.last_plain = Some(Instant::now());
            }
            return;
        };
        let bar = self.task.get_or_insert_with(|| display.add(ProgressBar::new_spinner()));
        bar.disable_steady_tick(); bar.reset();
        let total = total.filter(|&n| n != 0);
        if let Some(total) = total { bar.set_length(total); } else { bar.unset_length(); }
        let template = match (unit, total.is_some()) {
            (Unit::Bytes, true) => "{prefix} {wide_bar} {bytes}/{total_bytes} {bytes_per_sec} ETA {eta} {msg}",
            (Unit::Bytes, false) => "{spinner} {prefix} {bytes} {bytes_per_sec} {elapsed} {msg}",
            (Unit::Work, _) => "{spinner} {prefix} {elapsed} {msg}",
            (_, true) => "{prefix} {wide_bar} {pos}/{len} {elapsed} {msg}",
            (_, false) => "{spinner} {prefix} {pos} rows {elapsed} {msg}",
        };
        if let Ok(style) = ProgressStyle::with_template(template) { bar.set_style(style); }
        bar.set_prefix(if detail.is_empty() { stage.label().to_owned() } else { format!("{} ({detail})", stage.label()) }); bar.set_message("");
        bar.enable_steady_tick(Duration::from_millis(1000 / u64::from(STARTUP_PROGRESS_HZ.max(1))));
    }
    fn set_position(&mut self, completed: u64) { if let Some(bar) = &self.task { bar.set_position(completed); } }
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
impl<W: Write> Drop for StartupProgress<W> {
    fn drop(&mut self) { self.stop_display(); let _ = self.flush(); } // cleanup never claims that unfinished work succeeded
}

#[cfg(test)]
mod tests;
