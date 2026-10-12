//! No-op hooks left over from the removed memory-diagnostics feature: descriptors and diagnostic-only bodies are
//! never evaluated. The hooks and their call sites go away with the old processing spine.
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
}
