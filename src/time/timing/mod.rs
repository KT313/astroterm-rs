//! Measure startup and frame checkpoints and expose the opt-in execution trace.
//! Use this module's exports; recording and the trace implementation remain private.

/// Keep 95% of the previous timing average and add 5% of the current frame's duration.
const TIMING_EMA_FACTOR: f64 = 0.95;

mod pipeline;
mod checkpoints;
mod averages;
mod trace;
mod capture;
mod details;
pub use capture::CaptureSteps;
pub use trace::{PipelineTrace, TraceStep};


/// A startup total or the smoothed per-frame duration of one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepTime {
    pub name: &'static str,
    pub depth: usize,
    pub average_seconds: f64,
}

/// Timing totals and frame averages, grouped by their named checkpoint paths.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepTimes {
    steps: Vec<StepTime>,
    records: Vec<StepRecord>,
    per_frame: bool,
    trace: Option<PipelineTrace>,
    detailed: bool,
    checkpoints: Box<checkpoints::Checkpoints>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StepRecord {
    parents: Vec<&'static str>,
    initialized: bool,
    previous_average: Option<f64>,
    frame_seconds: Option<f64>,
}
