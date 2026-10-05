//! Typed observations of explicit boundaries; counts are logical payload, never physical traffic.
use crate::cache::{Cache, CacheStats};
use crate::timing::{StepTimes, memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}};

pub(super) fn snapshot_cache<K, V>(cache: &Cache<K, V>) -> (u64, CacheStats) { (cache.generation, cache.stats) }

/// A completed get_or_update already knows the decision. Read its counters, never compare its values again.
#[inline]
pub(super) fn record_cache<K, V>(times: &mut StepTimes, buffer: BufferId, before: Option<(u64, CacheStats)>, cache: &Cache<K, V>) {
    let Some((generation, stats)) = before else { return; };
    let step = times.last_memory_step();
    if cache.stats.hits != stats.hits {
        times.record_memory(step, || MemoryEvent::unknown_operation(buffer, Operation::Reuse));
    } else if let Some(reason) = cache.stats.last_reason {
        times.record_memory(step, || MemoryEvent::unknown_operation(buffer, Operation::Refresh(reason)));
        if cache.stats.refreshes != stats.refreshes {
            times.record_memory(step, || MemoryEvent::unknown_operation(buffer, Operation::Compare));
            times.record_memory(step, || MemoryEvent::unknown_operation(buffer, Operation::Store { value_changed: cache.generation != generation }));
        }
    }
}

/// Bounded reception-cache instrumentation. Comparing generations observes the existing store result, not values.
#[inline]
pub(super) fn record_observer_memory(times: &mut StepTimes, before: &crate::cache::CacheReport, after: &crate::cache::CacheReport, reuse: bool) {
    let step = times.last_memory_step();
    times.record_memory(step, || MemoryEvent::borrow(BufferId::ObserverGeometry, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
    if reuse && before.calculated_at.is_some() && !before.has_been_invalidated {
        times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::ObserverGeometry, Operation::Compare));
    }
    times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::ObserverGeometry,
        if after.stats.hits > before.stats.hits { Operation::Reuse } else { Operation::Refresh(after.stats.last_reason.expect("refresh has reason")) }));
    if after.stats.refreshes > before.stats.refreshes {
        times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::ObserverGeometry, Operation::Compare));
        times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::ObserverGeometry, Operation::Store { value_changed: after.generation != before.generation }));
    }
}


/// Describe a completed direction calculation, keeping the mutation itself in the named processing pass.
#[inline]
pub(super) fn record_direction_pass(times: &mut StepTimes, sky: &crate::model::ObservedSky) {
    times.record_borrow(BufferId::ObservedStars, Access::Writable, || BufferShape::vector(&sky.stars, IndexDomain::Observed));
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::ObservedStars, Operation::Write,
        None, None, Some(sky.stars.len()), sky.stars.len().checked_mul(std::mem::size_of::<crate::astro::Vector3>())));
}

#[inline]
pub(super) fn record_direction_capture(times: &mut StepTimes, buffer: BufferId, positions: &crate::model::observation::Directions) {
    times.record_memory(times.last_memory_step(), || {
        let count = positions.0.len() + positions.1.len() + 1;
        MemoryEvent::operation(buffer, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<crate::astro::Vector3>()))
    });
}

#[inline]
pub(super) fn record_direction_restoration(times: &mut StepTimes, buffer: BufferId, sky: &crate::model::ObservedSky) {
    times.record_unknown(buffer, Operation::Reuse);
    times.record_memory(times.last_memory_step(), || {
        let count = sky.stars.len() + sky.planets.len() + 1;
        MemoryEvent::operation(buffer, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<crate::astro::Vector3>()))
    });
}
