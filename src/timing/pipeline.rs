//! Run timed steps in execution order; detailed recording and retention live in private helpers.
use super::{StepTime, StepTimes};
use std::time::Instant;
#[cfg(feature = "memory-diagnostics")]
use super::run;

#[cfg(feature = "memory-diagnostics")]
use super::{MemoryFrame, MemoryRun, PipelineTrace};
#[cfg(feature = "memory-diagnostics")]
use super::run::accumulate_completed;
#[cfg(feature = "memory-diagnostics")]
use super::reporting::{write_completed_totals, write_retained_segments, write_run_header};
#[cfg(feature = "memory-diagnostics")]
use crate::cache::InventorySnapshot;
#[cfg(feature = "memory-diagnostics")]
use std::io::{self, Write};

impl StepTimes {
    /// Start a frame. Repeated calls within one scope contribute to one frame total before smoothing.
    /// Callers without a frame loop retain the original per-call averaging behavior.
    pub fn begin_frame(&mut self) {
        self.reset_frame_samples(); // carry completed averages forward and clear this frame's totals
        self.per_frame = true;
    }

    /// Run a step, add its duration to the step's average, and return its result.
    pub fn measure<T>(&mut self, name: &'static str, run: impl FnOnce() -> T) -> T {
        let trace_index = self.start_trace(name); // reserve the execution-order row before running the work
        let start = Instant::now();
        let result = run();
        let seconds = start.elapsed().as_secs_f64();
        self.finish_trace(trace_index, seconds); // retain this call's unsmoothed elapsed time
        self.record(name, seconds);              // update the timing panel's running average
        result
    }

    /// Measure an existing leaf pass, exposing context for gated memory events without adding ordinary scopes.
    /// The callback must not introduce nested ordinary timers; use measure_steps for a real parent stage.
    #[inline(always)]
    pub fn measure_with_memory<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        let index = self.start_trace(name);
        let start = Instant::now();
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_enabled { self.push_trace_scope(index); }
        let result = run(self);
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_enabled { self.pop_trace_scope(index); }
        let seconds = start.elapsed().as_secs_f64();
        self.finish_trace(index, seconds);
        self.record(name, seconds);
        result
    }

    /// Measure a stage that also records its own sub-steps.
    pub fn measure_steps<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        let trace_index = self.start_trace(name);
        self.register_step(name); // reserve the parent before its children, so the panel reads in pipeline order
        let start = Instant::now();
        #[cfg(feature = "memory-diagnostics")]
        let retain_parent = !self.memory_bounded || self.parents.len() < run::MAX_TRACE_DEPTH; // trace suppression does not bound this separate parent-path stack
        #[cfg(not(feature = "memory-diagnostics"))]
        let retain_parent = true;
        if retain_parent { self.parents.push(name); }
        self.push_trace_scope(trace_index);
        let result = run(self);
        self.pop_trace_scope(trace_index);
        if retain_parent { self.parents.pop(); }
        let seconds = start.elapsed().as_secs_f64();
        self.finish_trace(trace_index, seconds);
        self.record(name, seconds);
        result
    }

    /// Run normal processing, adding a trace-only scope only when memory events are requested.
    /// This does not add smoothed timing-panel rows. Feature-off callers reduce to the processing body.
    #[inline(always)]
    pub fn measure_memory_scope<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_enabled {
            let index = self.start_trace(name);
            let start = Instant::now();
            self.push_trace_scope(index);
            let result = run(self);
            self.pop_trace_scope(index);
            self.finish_trace(index, start.elapsed().as_secs_f64());
            return result;
        }
        #[cfg(not(feature = "memory-diagnostics"))]
        let _ = name;
        run(self)
    }

    /// Time bounded-batch passes without a clock per object or thousands of trace entries. Child rows are
    /// explicitly aggregated in first-occurrence order; their calls interleave once per batch.
    pub fn measure_batches<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        self.measure_steps(name, |times| {
            let mut batches = times.prepare_batch_times(); // collect repeated passes without one trace row per batch
            let result = run(&mut batches);
            times.record_batch_results(&batches);          // publish each pass once, with summed timing and events
            result
        })
    }

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }
}

#[cfg(feature = "memory-diagnostics")]
impl StepTimes {
    /// Enable after validation and before dataset loading. Existing startup trace records are preserved.
    pub fn enable_memory_run(&mut self, cache_enabled: bool) {
        self.prepare_bounded_history(); // cap any startup evidence captured before activation
        self.memory_epoch = self.memory_epoch.checked_add(1).expect("diagnostic frame handle epoch exhausted");
        self.memory_completed = None;
        self.memory_enabled = true;
        self.memory_bounded = true;
        self.memory_run = Some(MemoryRun::new(cache_enabled));
    }

    /// Separate startup from frame-local records without resetting normal smoothed timings.
    pub fn begin_memory_frame(&mut self) {
        let Some(run) = &mut self.memory_run else { return; };
        assert!(!run.frame_active, "complete or cancel the previous diagnostic frame");
        if run.startup.is_none() { run.startup = self.trace.take(); } // keep startup evidence apart from frames
        self.trace = Some(PipelineTrace::default());
        self.memory_completed = None;
        self.memory_epoch = self.memory_epoch.checked_add(1).expect("diagnostic frame handle epoch exhausted");
        self.memory_suppressed = 0;
        run.current_time = None;
        run.frame_active = true;
    }

    /// Record the frame's simulated time before any fallible processing.
    pub fn set_memory_frame_time(&mut self, utc: f64, tt: f64) {
        if let Some(run) = &mut self.memory_run { run.current_time = Some((utc, tt)); }
    }

    /// Commit only after successful presentation and the final inventory capture.
    pub fn complete_memory_frame(&mut self, elapsed_seconds: f64) {
        let Some(run) = &mut self.memory_run else { return; };
        assert!(run.frame_active, "begin the diagnostic frame first");
        let time = run.current_time;
        let mut trace = self.trace.take().unwrap_or_default();
        let start = std::time::Instant::now();
        accumulate_completed(run, &trace); // add successful frame counts and durations to run totals
        trace.unscoped_diagnostic_seconds += start.elapsed().as_secs_f64();
        run.latest = Some(MemoryFrame { utc: time.map(|t| t.0), tt: time.map(|t| t.1), elapsed_seconds, trace });
        run.completed_frames = run.completed_frames.saturating_add(1); // failed or cancelled frames never reach here
        run.frame_active = false;
        run.current_time = None;
        self.trace = Some(PipelineTrace::default());
        self.memory_completed = None;
        self.memory_epoch = self.memory_epoch.checked_add(1).expect("diagnostic frame handle epoch exhausted");
        self.memory_suppressed = 0;
    }

    /// A quit command did not present a frame; discard its empty pending record set.
    pub fn cancel_memory_frame(&mut self) {
        let Some(run) = &mut self.memory_run else { return; };
        run.frame_active = false;
        run.current_time = None;
        self.trace = Some(PipelineTrace::default());
        self.memory_completed = None;
        self.memory_epoch = self.memory_epoch.checked_add(1).expect("diagnostic frame handle epoch exhausted");
        self.memory_suppressed = 0;
    }

    pub fn memory_run(&self) -> Option<&MemoryRun> { self.memory_run.as_ref() }

    /// Read owned snapshots across startup, the last completed frame and any in-progress frame.
    pub fn memory_inventories(&self) -> impl Iterator<Item = &InventorySnapshot> {
        self.memory_run.iter().flat_map(|run| run.startup.iter().chain(run.latest.iter().map(|frame| &frame.trace)))
            .chain(self.trace.iter()).flat_map(|trace| trace.memory_snapshots.iter())
    }

    /// Format retained execution data only. Inventory formatting remains in the state layer.
    pub fn write_memory_run_report(&self, output: &mut impl Write) -> io::Result<()> {
        let Some(run) = &self.memory_run else { return Ok(()); };
        write_run_header(output, run, self.registry_omitted)?; // explain retained evidence and its limits
        write_completed_totals(output, run)?;                  // show sums and counts for completed frames
        write_retained_segments(output, run, &self.trace)?;    // print startup, latest frame and unfinished work
        Ok(())
    }
}
