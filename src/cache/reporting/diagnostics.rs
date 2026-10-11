//! Inspectable cache decisions, distinct from smoothed execution timings.
use super::{Cache, CacheStats};

#[derive(Clone, Debug, PartialEq)]
pub struct CacheReport {
    pub name: &'static str,
    pub calculated_at: Option<f64>,
    pub valid_seconds: f64,
    pub generation: u64,
    pub has_been_invalidated: bool,
    pub stats: CacheStats,
}
impl<K, V> Cache<K, V> {
    pub fn report(&self, name: &'static str) -> CacheReport {
        CacheReport {
            name,
            calculated_at: self.calculated_at,
            valid_seconds: self.valid_seconds,
            generation: self.generation,
            has_been_invalidated: self.has_been_invalidated,
            stats: self.stats,
        }
    }
}

/// Compact lifetime counters; hit/refresh/bypass counts remain separate from stage timing averages.
pub fn format_stats(stats: CacheStats) -> String {
    format!("H:{} R:{} B:{}", stats.hits, stats.refreshes, stats.bypasses)
}

/// How much of a described logical payload is known; independent of collector activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality { ExactPayload, LowerBound, Unknown }
