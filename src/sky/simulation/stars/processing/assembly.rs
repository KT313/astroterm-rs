//! Selected output is a dependency-only view materialization. Region freshness is checked before this step.
use crate::{cache::Group, state::StellarMotionBuffers, timing::{StepTimes, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}};

pub(super) fn assemble_selected_output(storage: &mut StellarMotionBuffers<'_>, epoch: f64, times: &mut StepTimes) {
    let key = (storage.key.0, storage.key.1, *storage.generation);
    let refresh = times.measure("Motion cache decision", || storage.motion.needs_refresh(&key, epoch, None, storage.config.allows(Group::StellarState)));
    times.record_candidate_decision(times.last_memory_step(), BufferId::MotionSamples, BufferId::MotionSamples, refresh, storage.motion.stats.last_reason);
    if !refresh { return; }
    let (values, singular) = times.measure("Motion output assembly", || {
        let mut values = Vec::with_capacity(storage.working.value().len());
        let mut singular = 0;
        let mut rows = storage.working.value().iter().peekable();
        for &region in storage.requested_regions {
            let (start, end) = (storage.offsets[region], storage.offsets[region + 1]);
            let samples = storage.regions.entries[region].value(); // one validity assertion per region, never per star
            assert_eq!(samples.len(), end - start, "region must be completely populated");
            while rows.peek().is_some_and(|row| row.source_index < end) {
                let row = rows.next().unwrap();
                assert!(row.source_index >= start, "selected star belongs to an unrequested region");
                let sample = &samples[row.source_index - start];
                values.push((sample.direction, sample.magnitude));
                singular += usize::from(sample.used_singular_fallback);
            }
        }
        assert!(rows.next().is_none(), "all selected stars must have regional samples");
        (values, singular)
    });
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::MotionSamples, Operation::Append, None,
        Some(BufferShape::vector(&values, IndexDomain::Working)), Some(values.len()), values.len().checked_mul(std::mem::size_of::<(crate::astro::Vector3, f64)>())));
    times.describe("Motion output assembly", || format!("output selected stars={}; gathered from complete regions without per-star cache lookup; singular fallbacks={singular}", values.len()));
    let outcome = times.measure("Motion cache store", || storage.motion.store(key, epoch, 0.0, (values, singular)));
    times.record_store(BufferId::MotionSamples, outcome);
}
