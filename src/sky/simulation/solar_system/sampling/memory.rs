//! Logical sample-family operations, shared by planetary, lunar and orientation preparation.
use crate::model::Sample;
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, MemoryStepId, Operation};

/// `step` is the measured step the events belong to: the last completed step when called after `measure`,
/// the active step when called inside `measure_with_memory` (the per-body planetary loop).
#[allow(clippy::ptr_arg)] // retained capacity is part of the observation
#[inline]
pub(super) fn record_sample_family<T>(times: &mut StepTimes, step: Option<MemoryStepId>, buffers: (BufferId, BufferId), before: Option<(BufferShape, BufferShape)>, values: (&Vec<Sample<T>>, &Vec<Sample<T>>), new_samples: u64, rebuilt: Option<bool>) {
    let (buffer, work_id) = buffers;
    let (samples, work) = values;
    let work_before = before.map(|pair| pair.1);
    let before = before.map(|pair| pair.0);
    times.with_memory(|times| {
        if let Some(shape) = before { times.record_memory(step, || MemoryEvent::borrow(buffer, Access::Writable, shape)); }
        let Some(rebuilt) = rebuilt else { return; }; // a failed preparation did not publish its work buffer
        if !rebuilt {
            times.record_memory(step, || MemoryEvent::operation(buffer, Operation::Reuse, before, before, Some(samples.len()), None));
            return;
        }
        if let Some(shape) = work_before { times.record_memory(step, || MemoryEvent::borrow(work_id, Access::Writable, shape)); }
        let prepared = BufferShape::vector(samples, IndexDomain::ModelSamples); // this allocation belonged to work before the swap
        if work_before.is_some_and(|shape| prepared.capacity > shape.capacity) {
            times.record_memory(step, || MemoryEvent::operation(work_id, Operation::Reserve, work_before, Some(prepared), None, None));
        }
        times.record_memory(step, || {
            let count = new_samples as usize;
            MemoryEvent::operation(work_id, Operation::Build, None, None, Some(count), count.checked_mul(std::mem::size_of::<Sample<T>>()))
        });
        times.record_memory(step, || {
            let count = samples.len() - new_samples as usize;
            MemoryEvent::operation(work_id, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<Sample<T>>()))
        });
        times.record_memory(step, || {
            let shape = BufferShape::vector(samples, IndexDomain::ModelSamples);
            MemoryEvent::operation(buffer, Operation::Move, before, Some(shape), shape.len, None)
        });
        times.record_memory(step, || MemoryEvent::operation(work_id, Operation::Move, work_before,
            Some(BufferShape::vector(work, IndexDomain::ModelSamples)), Some(0), None)); // displaced allocation was cleared, not freed
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
        let before = times.inspect_memory(|| (BufferShape::vector(&samples, IndexDomain::ModelSamples), BufferShape::vector(&samples, IndexDomain::ModelSamples)));
        let step = times.last_memory_step();
        record_sample_family(&mut times, step, (BufferId::OrientationSamples, BufferId::OrientationSampleWork), before, (&samples, &samples), 0, Some(true));
        let events = &times.trace().unwrap().steps[0].memory_events;
        assert_eq!(events.len(), 6);
        assert!(matches!(events[2].event, MemoryEvent::Operation { operation: Operation::Build, elements: Some(0), logical_bytes: Some(0), .. }));
        assert!(matches!(events[3].event, MemoryEvent::Operation { operation: Operation::Copy, elements: Some(0), logical_bytes: Some(0), .. }));
    }
}
