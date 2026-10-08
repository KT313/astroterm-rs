//! Shared lazy hooks. Domain helpers compose these without owning a second event registry.
use super::{Access, BufferId, BufferShape, MemoryEvent, MemoryStepId, Operation, StepTimes};

impl StepTimes {
    /// Run a diagnostic-only group only with memory collection enabled; never put processing work here.
    #[inline(always)]
    pub fn with_memory(&mut self, inspect: impl FnOnce(&mut Self)) {
        if self.memory_events_enabled() { inspect(self); }
    }

    #[inline(always)]
    pub fn record_borrow(&mut self, buffer: BufferId, access: Access, shape: impl FnOnce() -> BufferShape) {
        self.record_memory(self.last_memory_step(), || MemoryEvent::borrow(buffer, access, shape()));
    }

    #[inline(always)]
    pub fn record_build(&mut self, buffer: BufferId, shape: impl FnOnce() -> BufferShape) {
        self.record_memory(self.last_memory_step(), || {
            let shape = shape();
            MemoryEvent::operation(buffer, Operation::Build, None, Some(shape), shape.len, shape.logical_bytes())
        });
    }

    #[inline(always)]
    pub fn record_shape(&mut self, buffer: BufferId, operation: Operation, before: Option<BufferShape>, shape: impl FnOnce() -> BufferShape) {
        self.record_memory(self.last_memory_step(), || {
            let after = shape();
            let bytes = if matches!(operation, Operation::Copy | Operation::Write | Operation::Output) { after.logical_bytes() } else { None };
            let elements = if matches!(operation, Operation::Clear | Operation::Move) { before.and_then(|shape| shape.len).or(after.len) } else { after.len };
            MemoryEvent::operation(buffer, operation, before, Some(after), elements, bytes)
        });
    }

    #[inline(always)]
    pub fn record_unknown(&mut self, buffer: BufferId, operation: Operation) {
        self.record_memory(self.last_memory_step(), || MemoryEvent::unknown_operation(buffer, operation));
    }

    #[inline(always)]
    pub fn record_store(&mut self, buffer: BufferId, outcome: crate::cache::StoreOutcome) {
        self.record_unknown(buffer, Operation::Compare);
        self.record_unknown(buffer, Operation::Store { value_changed: outcome.value_changed });
    }

    /// Aggregate region decisions without one diagnostic event per region; element counts mean regions.
    #[inline(always)]
    pub fn record_regional_counts(&mut self, buffer: BufferId, before: crate::cache::CacheStats, after: crate::cache::CacheStats) {
        self.with_memory(|times| {
            let step = times.last_memory_step();
            let hits = after.hits - before.hits;
            let refreshes = after.refreshes - before.refreshes;
            if hits > 0 { times.record_memory(step, || MemoryEvent::operation(buffer, Operation::Reuse, None, None, Some(hits as usize), None)); }
            if refreshes > 0 { times.record_memory(step, || MemoryEvent::operation(buffer, Operation::Build, None, None, Some(refreshes as usize), None)); }
        });
    }

    /// Attach to the supplied active or completed step; never compare cached values a second time.
    /// A missing refresh reason is reported as unknown, not invented or treated as a processing failure.
    #[inline(always)]
    pub fn record_candidate_decision(&mut self, step: Option<MemoryStepId>, candidate: BufferId, output: BufferId, refresh: bool, reason: Option<crate::cache::RefreshReason>) {
        self.with_memory(|times| {
            use crate::cache::RefreshReason;
            if !refresh || matches!(reason, Some(RefreshReason::Dependencies | RefreshReason::Expired)) {
                times.record_memory(step, || MemoryEvent::unknown_operation(candidate, Operation::Compare));
            }
            times.record_memory(step, || MemoryEvent::unknown_operation(output,
                if refresh { reason.map_or(Operation::RefreshUnknown, Operation::Refresh) } else { Operation::Reuse }));
        });
    }
}
