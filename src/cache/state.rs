//! One result per dependency key, with explicit invalidation and non-sliding validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshReason {
    Missing,
    Invalidated,
    Dependencies,
    Expired,
    Bypassed,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub refreshes: u64,
    pub bypasses: u64,
    pub last_reason: Option<RefreshReason>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Cache<K, V> {
    pub has_been_invalidated: bool,
    pub calculated_at: Option<f64>,
    pub valid_seconds: f64,
    pub generation: u64,
    pub stats: CacheStats,
    key: Option<K>,
    value: Option<V>,
}
impl<K, V> Default for Cache<K, V> {
    fn default() -> Self {
        Self {
            has_been_invalidated: true,
            calculated_at: None,
            valid_seconds: 0.0,
            generation: 0,
            stats: CacheStats::default(),
            key: None,
            value: None,
        }
    }
}
impl<K: PartialEq, V: PartialEq> Cache<K, V> {
    pub fn invalidate(&mut self) {
        self.has_been_invalidated = true;
    }
    pub fn value(&self) -> &V {
        assert!(!self.has_been_invalidated, "cache must be refreshed before reading");
        self.value.as_ref().expect("cache prepared before reading")
    }
    /// None age is dependency-only. Zero age allows only exactly matching epochs; callers disable reuse for policy zero.
    pub fn needs_refresh(&mut self, key: &K, epoch: f64, age: Option<f64>, enabled: bool) -> bool {
        let reason = if !enabled {
            Some(RefreshReason::Bypassed)
        } else if self.value.is_none() {
            Some(RefreshReason::Missing)
        } else if self.has_been_invalidated {
            Some(RefreshReason::Invalidated)
        } else if self.key.as_ref() != Some(key) {
            Some(RefreshReason::Dependencies)
        } else if age
            .is_some_and(|age| (epoch - self.calculated_at.unwrap()).abs() > age.min(self.valid_seconds) / 86400.0)
        {
            Some(RefreshReason::Expired)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.stats.last_reason = Some(reason);
            self.stats.bypasses += u64::from(reason == RefreshReason::Bypassed);
            self.has_been_invalidated = true;
            true
        } else {
            self.stats.hits += 1;
            false
        }
    }
    pub fn store(&mut self, key: K, epoch: f64, valid_seconds: f64, value: V) {
        if self.value.as_ref() != Some(&value) {
            self.generation = self.generation.wrapping_add(1);
        }
        self.key = Some(key);
        self.value = Some(value);
        self.calculated_at = Some(epoch);
        self.valid_seconds = valid_seconds;
        self.has_been_invalidated = false;
        self.stats.refreshes += 1;
    }
    pub fn get_or_update(&mut self, key: K, epoch: f64, enabled: bool, calculate: impl FnOnce() -> V) -> &V {
        if self.needs_refresh(&key, epoch, None, enabled) {
            self.store(key, epoch, 0.0, calculate());
        }
        self.value()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_results_do_not_invalidate_downstream_generations() {
        let mut cache = Cache::default();
        cache.get_or_update(1, 10.0, true, || vec![3, 4]);
        let version = cache.generation;
        cache.get_or_update(2, 11.0, true, || vec![3, 4]);
        assert_eq!(cache.generation, version);
        assert_eq!(cache.calculated_at, Some(11.0));
        cache.get_or_update(3, 12.0, true, || vec![4, 5]);
        assert_ne!(cache.generation, version);
    }

    #[test]
    fn validity_is_symmetric_non_sliding_and_dependency_guarded() {
        let mut c = Cache::default();
        let t = 2451545.0;
        assert!(c.needs_refresh(&1, t, Some(60.0), true));
        c.store(1, t, 60.0, 42);
        for seconds in [-30.0, 0.0, 30.0] {
            assert!(!c.needs_refresh(&1, t + seconds / 86400.0, Some(60.0), true));
        }
        assert_eq!(c.calculated_at, Some(t));
        assert!(c.needs_refresh(&1, t + 61.0 / 86400.0, Some(60.0), true));
        assert_eq!(c.stats.last_reason, Some(RefreshReason::Expired));
        assert!(c.has_been_invalidated); // a failed refresh cannot turn a stale entry valid
        c.store(1, t, 60.0, 43);
        assert!(c.needs_refresh(&2, t, None, true));
        c.store(2, t, 60.0, 43);
        assert!(c.needs_refresh(&2, t, None, false));
        c.store(2, t, 60.0, 43);
        c.invalidate();
        assert!(c.needs_refresh(&2, t, None, true));
    }
}
