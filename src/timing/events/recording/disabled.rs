//! Compile-time no-op hooks: descriptors and diagnostic-only bodies are never evaluated.
use super::{MemoryEvent, MemoryStepId, StepTimes};

impl StepTimes {
    #[inline(always)]
    pub fn memory_events_enabled(&self) -> bool { false }
    #[inline(always)]
    pub fn last_memory_step(&self) -> Option<MemoryStepId> { None }
    #[inline(always)]
    pub fn active_memory_step(&self) -> Option<MemoryStepId> { None }
    #[inline(always)]
    pub fn inspect_memory<T>(&mut self, _inspect: impl FnOnce() -> T) -> Option<T> { None }
    #[inline(always)]
    pub fn record_memory(&mut self, _id: Option<MemoryStepId>, _event: impl FnOnce() -> MemoryEvent) {}
    #[inline(always)]
    pub fn begin_memory_frame(&mut self) {}
    #[inline(always)]
    pub fn set_memory_frame_time(&mut self, _utc: f64, _tt: f64) {}
    #[inline(always)]
    pub fn complete_memory_frame(&mut self, _elapsed_seconds: f64) {}
    #[inline(always)]
    pub fn cancel_memory_frame(&mut self) {}
}
