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

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }
}
