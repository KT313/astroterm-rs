//! Measure frame steps and expose the opt-in execution trace.
//! Use this module's exports; recording, history and event implementation remain private.
use crate::rows::row_columns;

mod pipeline;
#[path = "recording/averages.rs"]
mod averages;
#[path = "recording/batches.rs"]
mod batches;
#[path = "history/trace.rs"]
mod trace;
#[path = "events/mod.rs"]
mod memory;
#[path = "reporting/formatting.rs"]
mod formatting;
#[path = "reporting/capture.rs"]
mod capture;
pub use capture::CaptureSteps;

pub use trace::{PipelineTrace, TraceStep};
pub use memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, MemoryStepId, Operation};
pub(crate) use formatting::format_bytes;


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
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StepRecord {
    parents: Vec<&'static str>,
    initialized: bool,
    previous_average: Option<f64>,
    frame_seconds: Option<f64>,
    frame_calls: usize,
}
