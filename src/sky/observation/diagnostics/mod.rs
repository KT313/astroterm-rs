//! Bounded logical events for authoritative direction caches and their reusable work buffers.
use crate::{model::Directions, timing::{StepTimes, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}};

pub(crate) fn direction_shape(values: &Directions) -> BufferShape {
    BufferShape { len: Some(values.0.len() + values.1.len() + 1), capacity: Some(values.0.capacity() + values.1.capacity() + 1),
        element_bytes: Some(std::mem::size_of::<crate::astro::Vector3>()), domain: IndexDomain::Unknown,
        quality: crate::cache::Quality::ExactPayload }
} // logical slots across two vectors and the inline Moon, not a claim of contiguous storage

pub(crate) fn record_direction_commit(times: &mut StepTimes, result: BufferId, work_id: BufferId,
    completed: Option<BufferShape>, displaced: Option<BufferShape>, work: &Directions, outcome: crate::cache::StoreOutcome,
) {
    times.record_store(result, outcome);
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(result, Operation::Move, None, completed,
        completed.and_then(|shape| shape.len), completed.and_then(|shape| shape.logical_bytes())));
    times.record_shape(work_id, Operation::Clear, displaced, || direction_shape(work));
}
