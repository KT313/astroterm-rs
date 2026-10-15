//! Self-contained simulation clocks, frame pacing and diagnostic timing.
//! Callers supply plain settings; this module depends only on the standard library, chrono and serde.
mod calendar;
mod simulation_clock;
mod time_manager;
mod timing;

pub use simulation_clock::SimulationClock;
pub use time_manager::{StartupTiming, TimeManager};
pub use timing::{CaptureSteps, PipelineTrace, StepTime, StepTimes, TraceStep};

pub use calendar::{MILLISECONDS_PER_SECOND, MILLISECONDS_PER_DAY, UNIX_EPOCH_TIME_MS, current_time_ms, datetime_to_time_ms, parse_simulation_time_ms, time_ms_to_utc, parse_seconds_to_ms, format_ms_as_seconds};
