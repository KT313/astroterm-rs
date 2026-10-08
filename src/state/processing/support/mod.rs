//! Small ownership/provenance helpers, with no processing or buffer allocation.
#[cfg(feature = "memory-diagnostics")]
pub(super) mod regions;
pub(super) fn sum_stats(stats: impl IntoIterator<Item = crate::cache::CacheStats>) -> crate::cache::CacheStats {
    let mut total = crate::cache::CacheStats::default();
    for stats in stats { total.hits += stats.hits; total.refreshes += stats.refreshes; total.bypasses += stats.bypasses; }
    total
}

/// Cache generations are local to an owner; this token prevents accidental reuse across different owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StageId(u64);
impl StageId { pub(super) fn value(self) -> u64 { self.0 } }
impl Default for StageId {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |id| id.checked_add(1)).expect("stage identity exhausted"))
    }
}


#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(StageId);
