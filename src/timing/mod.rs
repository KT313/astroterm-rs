//! Measure frame steps and expose the opt-in execution trace.
//! Use this module's exports; recording and the trace implementation remain private.
use crate::rows::row_columns;

mod pipeline;
mod averages;
mod trace;
mod capture;
mod details;
pub use details::DetailBatch;
pub use capture::CaptureSteps;
pub use trace::{PipelineTrace, TraceStep};


/// The smoothed duration of one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepTime {
    pub name: &'static str,
    pub depth: usize,
    pub average_seconds: f64,
}
row_columns!(StepTime { name, depth, average_seconds });

/// Smoothed durations of named steps, one exponential moving average per step, in the order the steps first ran.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepTimes {
    steps: Vec<StepTime>,
    records: Vec<StepRecord>,
    parents: Vec<&'static str>,
    per_frame: bool,
    trace: Option<PipelineTrace>,
    detailed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StepRecord {
    parents: Vec<&'static str>,
    initialized: bool,
    previous_average: Option<f64>,
    frame_seconds: Option<f64>,
    frame_calls: usize,
}
