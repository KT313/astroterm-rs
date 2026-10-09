//! Candidate lifecycle observations; calculations and cache mutations stay in the projection passes.
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};

#[allow(clippy::ptr_arg)] // record retained capacity as well as logical length
#[inline]
pub(super) fn record_key_build<T>(times: &mut StepTimes, buffer: BufferId, before: Option<BufferShape>, values: &Vec<T>, domain: IndexDomain) {
    let step = times.last_memory_step();
    times.record_memory(step, || MemoryEvent::operation(buffer, Operation::Clear, before, before.map(|mut shape| { shape.len = Some(0); shape }), before.and_then(|shape| shape.len), None));
    times.record_memory(step, || {
        let shape = BufferShape::vector(values, domain);
        MemoryEvent::operation(buffer, Operation::Build, before.map(|mut shape| { shape.len = Some(0); shape }), Some(shape), shape.len, shape.logical_bytes())
    });
}

#[inline]
pub(super) fn record_cache_store(times: &mut StepTimes, candidate: BufferId, output: BufferId, before: Option<BufferShape>, after: impl FnOnce() -> BufferShape, outcome: crate::cache::StoreOutcome) {
    let step = times.last_memory_step();
    times.record_memory(step, || MemoryEvent::unknown_operation(output, Operation::Compare)); // existing Option/value comparison; no extra scan
    times.record_memory(step, || MemoryEvent::operation(candidate, Operation::Move, before, Some(after()), before.and_then(|shape| shape.len), Some(0))); // ownership moves, element payload is not copied
    times.record_memory(step, || MemoryEvent::unknown_operation(output, Operation::Store { value_changed: outcome.value_changed }));
}

#[allow(clippy::ptr_arg)]
#[inline]
pub(super) fn record_candidate_clear<T>(times: &mut StepTimes, buffer: BufferId, before: Option<BufferShape>, values: &Vec<T>, domain: IndexDomain) {
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(buffer, Operation::Clear, before, Some(BufferShape::vector(values, domain)), before.and_then(|shape| shape.len), None));
}

#[allow(clippy::ptr_arg)] // retained work capacity is part of the swap report
pub(super) fn record_region_store<T>(times: &mut StepTimes, work_id: BufferId, result_id: BufferId, transition: Option<(BufferShape, Option<BufferShape>)>, work: &Vec<T>, domain: IndexDomain, outcome: crate::cache::StoreOutcome) {
    let completed = transition.map(|(new, _)| new);
    let displaced = transition.and_then(|(_, old)| old);
    times.record_store(result_id, outcome);                                                // use the store's comparison result without scanning again
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(result_id, Operation::Move, None, completed,
        completed.and_then(|shape| shape.len), completed.and_then(|shape| shape.logical_bytes())));
    times.record_shape(work_id, Operation::Clear, displaced, || BufferShape::vector(work, domain)); // displaced result allocation is now empty scratch
}
