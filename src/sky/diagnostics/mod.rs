//! Typed observations of explicit boundaries; counts are logical payload, never physical traffic.
use crate::cache::{Cache, CacheStats};
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};

pub(crate) fn snapshot_cache<K, V>(cache: &Cache<K, V>) -> (u64, CacheStats) { (cache.generation, cache.stats) }

/// A completed get_or_update already knows the decision. Read its counters, never compare its values again.
#[inline]
pub(crate) fn record_cache<K, V>(times: &mut StepTimes, buffer: BufferId, before: Option<(u64, CacheStats)>, cache: &Cache<K, V>) {
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
pub(crate) fn record_observer_memory(times: &mut StepTimes, before: &crate::cache::CacheReport, after: &crate::cache::CacheReport, reuse: bool) {
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



/// Attach cache deltas within the producer's active parent, without relying on report-vector positions.
pub(crate) fn describe_cache_reports(previous: Option<Vec<crate::cache::CacheReport>>, current: impl FnOnce() -> Vec<crate::cache::CacheReport>, times: &mut StepTimes) {
    let Some(previous) = previous else { return; };
    times.measure_diagnostics(|times| {
        for (before, after) in previous.into_iter().zip(current()) {
            times.describe(after.name, || format!("cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; stored TT={:?}; validity={} s", after.stats.hits - before.stats.hits, after.stats.refreshes - before.stats.refreshes, after.stats.bypasses - before.stats.bypasses, after.stats.last_reason, after.calculated_at, after.valid_seconds));
        }
    });
}
