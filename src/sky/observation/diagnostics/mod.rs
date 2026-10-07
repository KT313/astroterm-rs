//! Direction-pass events belonging to observation.
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
/// Describe a completed direction calculation, keeping the mutation itself in the named processing pass.
#[inline]
pub(crate) fn record_direction_pass(times: &mut StepTimes, sky: &crate::model::ObservedSky) {
    times.record_borrow(BufferId::ObservedStars, Access::Writable, || BufferShape::vector(&sky.stars, IndexDomain::Observed));
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::ObservedStars, Operation::Write,
        None, None, Some(sky.stars.len()), sky.stars.len().checked_mul(std::mem::size_of::<crate::astro::Vector3>())));
}

#[inline]
pub(crate) fn record_direction_capture(times: &mut StepTimes, buffer: BufferId, positions: &crate::model::Directions) {
    times.record_memory(times.last_memory_step(), || {
        let count = positions.0.len() + positions.1.len() + 1;
        MemoryEvent::operation(buffer, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<crate::astro::Vector3>()))
    });
}

#[inline]
pub(crate) fn record_direction_restoration(times: &mut StepTimes, buffer: BufferId, sky: &crate::model::ObservedSky) {
    times.record_unknown(buffer, Operation::Reuse);
    times.record_memory(times.last_memory_step(), || {
        let count = sky.stars.len() + sky.planets.len() + 1;
        MemoryEvent::operation(buffer, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<crate::astro::Vector3>()))
    });
}
