//! Logical sample-family operations, shared by planetary, lunar and orientation preparation.
use crate::{model::simulation::Sample, timing::{StepTimes, memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}}};

#[allow(clippy::ptr_arg)] // retained capacity is part of the observation
#[inline]
pub(super) fn record_sample_family<T>(times: &mut StepTimes, buffer: BufferId, before: Option<BufferShape>, samples: &Vec<Sample<T>>, new_samples: u64, succeeded: bool) {
    times.with_memory(|times| {
        if let Some(shape) = before { times.record_borrow(buffer, Access::Writable, || shape); }
        if !succeeded { return; } // failed preparation did not transfer the prepared vector
        times.record_memory(times.last_memory_step(), || {
            let count = new_samples as usize;
            MemoryEvent::operation(buffer, Operation::Build, None, None, Some(count), count.checked_mul(std::mem::size_of::<Sample<T>>()))
        });
        times.record_memory(times.last_memory_step(), || {
            let count = samples.len() - new_samples as usize;
            MemoryEvent::operation(buffer, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<Sample<T>>()))
        });
        times.record_memory(times.last_memory_step(), || {
            let shape = BufferShape::vector(samples, IndexDomain::ModelSamples);
            MemoryEvent::operation(buffer, Operation::Move, before, Some(shape), shape.len, None)
        });
    });
}

#[cfg(all(test, feature = "memory-diagnostics"))]
mod tests {
    use super::*;
    #[test]
    fn empty_sample_family_reports_zero_without_indexing_a_sample() {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_events(true);
        times.measure("Samples", || ());
        let samples: Vec<Sample<crate::astro::Matrix3>> = Vec::new();
        let before = times.inspect_memory(|| BufferShape::vector(&samples, IndexDomain::ModelSamples));
        record_sample_family(&mut times, BufferId::OrientationSamples, before, &samples, 0, true);
        let events = &times.trace().unwrap().steps[0].memory_events;
        assert_eq!(events.len(), 4);
        assert!(matches!(events[1].event, MemoryEvent::Operation { operation: Operation::Build, elements: Some(0), logical_bytes: Some(0), .. }));
        assert!(matches!(events[2].event, MemoryEvent::Operation { operation: Operation::Copy, elements: Some(0), logical_bytes: Some(0), .. }));
    }
}
