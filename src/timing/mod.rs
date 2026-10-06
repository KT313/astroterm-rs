//! Measure frame steps and expose opt-in, bounded execution and memory diagnostics.
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
#[cfg(feature = "memory-diagnostics")]
#[path = "history/run.rs"]
mod run;
#[path = "reporting/formatting.rs"]
mod formatting;
#[cfg(feature = "memory-diagnostics")]
#[path = "reporting/run.rs"]
mod reporting;

pub use trace::{PipelineTrace, TraceStep};
pub use memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, MemoryStepId, Operation};
#[cfg(feature = "memory-diagnostics")]
pub use memory::{RecordedMemoryEvent, MAX_MEMORY_EVENTS_PER_STEP};
#[cfg(feature = "memory-diagnostics")]
pub use run::{MemoryFrame, MemoryRun, StepAggregate, TraceBounds, MAX_TRACE_STEPS, MAX_TRACE_DEPTH,
    MAX_TRACE_EVENTS, MAX_TRACE_DETAILS, MAX_TRACE_TEXT_BYTES, MAX_DETAIL_BYTES, MAX_TRACE_INVENTORIES,
    MAX_TIMING_PATHS, MAX_AGGREGATE_PATHS};
pub(crate) use formatting::format_bytes;
#[cfg(feature = "memory-diagnostics")]
pub(crate) use formatting::format_count;

/// Weight of the previous average in the exponential moving average; the newest frame gets the rest.
const EMA_FACTOR: f64 = 0.95;

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
    #[cfg(feature = "memory-diagnostics")]
    memory_enabled: bool,
    #[cfg(feature = "memory-diagnostics")]
    memory_completed: Option<memory::MemoryStepId>,
    #[cfg(feature = "memory-diagnostics")]
    memory_batch: bool,
    #[cfg(feature = "memory-diagnostics")]
    memory_batch_seconds: f64,
    #[cfg(feature = "memory-diagnostics")]
    memory_run: Option<MemoryRun>,
    #[cfg(feature = "memory-diagnostics")]
    memory_epoch: u64,
    #[cfg(feature = "memory-diagnostics")]
    memory_bounded: bool,
    #[cfg(feature = "memory-diagnostics")]
    memory_suppressed: usize,
    #[cfg(feature = "memory-diagnostics")]
    registry_omitted: u64,
    #[cfg(feature = "memory-diagnostics")]
    memory_batch_events: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StepRecord {
    parents: Vec<&'static str>,
    initialized: bool,
    previous_average: Option<f64>,
    frame_seconds: Option<f64>,
    frame_calls: usize,
    #[cfg(feature = "memory-diagnostics")]
    memory_events: Vec<memory::RecordedMemoryEvent>,
    #[cfg(feature = "memory-diagnostics")]
    memory_omitted: usize,
}
