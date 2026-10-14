//! Self-contained simulation clocks, frame pacing and diagnostic timing.
//! Callers supply plain settings; this module depends only on the standard library and serde.
mod simulation_clock;
mod time_manager;
mod timing;

pub use simulation_clock::SimulationClock;
pub use time_manager::{StartupTiming, TimeManager};
pub use timing::{CaptureSteps, PipelineTrace, StepTime, StepTimes, TraceStep};
