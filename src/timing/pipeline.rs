//! Run timed steps in execution order; detailed recording and retention live in private helpers.
use super::{StepTime, StepTimes};
use std::time::Instant;

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
        let result = run(self);
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

    /// Allocated slots in the original timing table, including spare capacity.
    pub(crate) fn step_capacity(&self) -> usize { self.steps.capacity() }

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }
}
