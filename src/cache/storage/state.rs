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
/// Result of the existing value comparison; callers may ignore it without a second comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreOutcome { pub value_changed: bool }
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
impl<K, V> Cache<K, V> {
    /// The stored result, even when invalidated or expired: an invalidated entry still owns its allocation.
    pub fn stored(&self) -> Option<&V> {
        self.value.as_ref()
    }
    /// The key the stored result was calculated for, if any.
    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
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
    /// None age is dependency-only. Zero age allows only exactly matching epochs; enabled controls explicit bypass separately.
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
    pub fn store(&mut self, key: K, epoch: f64, valid_seconds: f64, value: V) -> StoreOutcome {
        let value_changed = self.value.as_ref() != Some(&value);
        if value_changed {
            self.generation = self.generation.wrapping_add(1);
        }
        self.key = Some(key);
        self.value = Some(value);
        self.calculated_at = Some(epoch);
        self.valid_seconds = valid_seconds;
        self.has_been_invalidated = false;
        self.stats.refreshes += 1;
        StoreOutcome { value_changed }
    }
    pub fn get_or_update(&mut self, key: K, epoch: f64, enabled: bool, calculate: impl FnOnce() -> V) -> &V {
        if self.needs_refresh(&key, epoch, None, enabled) {
            self.store(key, epoch, 0.0, calculate());
        }
        self.value()
    }
}
impl<K: PartialEq, T: PartialEq> Cache<K, Vec<T>> {
    /// Publish complete work without copying elements; keep the displaced allocation for the next refresh.
    /// Call needs_refresh before filling work so a failed calculation leaves the old payload invalid.
    pub fn store_reusing(&mut self, key: K, epoch: f64, valid_seconds: f64, work: &mut Vec<T>) -> StoreOutcome {
        let value_changed = self.value.as_ref() != Some(work);
        if value_changed { self.generation = self.generation.wrapping_add(1); }
        if let Some(stored) = &mut self.value { std::mem::swap(stored, work); }
        else { self.value = Some(std::mem::take(work)); }
        self.key = Some(key);
        self.calculated_at = Some(epoch);
        self.valid_seconds = valid_seconds;
        self.has_been_invalidated = false;
        self.stats.refreshes += 1;
        work.clear(); // retained capacity belongs to the caller again
        StoreOutcome { value_changed }
    }
}
impl<K: PartialEq, T: PartialEq + Default> Cache<K, (Vec<T>, Vec<T>, T)> {
    /// Publish a completed two-vector result and scalar without copying either payload.
    pub fn store_reusing_pair(&mut self, key: K, epoch: f64, valid_seconds: f64, work: &mut (Vec<T>, Vec<T>, T)) -> StoreOutcome {
        let value_changed = self.value.as_ref() != Some(work);
        if value_changed { self.generation = self.generation.wrapping_add(1); }
        if let Some(stored) = &mut self.value { std::mem::swap(stored, work); }
        else { self.value = Some(std::mem::take(work)); }
        self.key = Some(key);
        self.calculated_at = Some(epoch);
        self.valid_seconds = valid_seconds;
        self.has_been_invalidated = false;
        self.stats.refreshes += 1;
        work.0.clear(); work.1.clear(); work.2 = T::default();
        StoreOutcome { value_changed }
    }
}
#[cfg(feature = "memory-diagnostics")]
impl<K: super::buffers::ReportBuffers, V: super::buffers::ReportBuffers> super::buffers::ReportBuffers for Cache<K, V> {
    const HAS_BUFFERS: bool = K::HAS_BUFFERS || V::HAS_BUFFERS;
    fn report_buffers(&self, sink: &mut dyn super::buffers::BufferSink) {
        super::buffers::report_field(sink, "key", &self.key);
        super::buffers::report_field(sink, "value", &self.value); // invalidated results still own their allocations
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

#[cfg(test)]
mod reuse_tests {
    use super::*;

    #[test]
    fn swapping_work_matches_store_metadata_and_keeps_both_allocations() {
        let mut cache = Cache::default();
        let mut reference = Cache::default();
        let mut work = Vec::with_capacity(8);
        for (key, time, values, enabled) in [(1, 10.0, vec![1, 2], true), (2, 11.0, vec![1, 2], true), (2, 12.0, vec![3, 4], false)] {
            assert_eq!(cache.needs_refresh(&key, time, Some(0.0), enabled), reference.needs_refresh(&key, time, Some(0.0), enabled));
            work.extend_from_slice(&values);
            let incoming = (work.as_ptr(), work.capacity());
            let outgoing = cache.stored().map(|v: &Vec<i32>| (v.as_ptr(), v.capacity()));
            assert_eq!(cache.store_reusing(key, time, 0.0, &mut work), reference.store(key, time, 0.0, values));
            assert_eq!(cache, reference);
            assert_eq!((cache.value().as_ptr(), cache.value().capacity()), incoming);
            if let Some(outgoing) = outgoing { assert_eq!((work.as_ptr(), work.capacity()), outgoing); }
            assert!(work.is_empty());
        }
        assert_eq!(cache.generation, 2);
        assert_eq!(cache.stats.refreshes, 3);
    }

    #[test]
    fn failed_work_preserves_invalid_old_payload_then_retries_against_that_payload() {
        let mut cache = Cache::default();
        cache.store((), 1.0, 0.0, vec![1, 2]);
        let original = cache.value().as_ptr();
        let mut work = Vec::with_capacity(8);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(cache.needs_refresh(&(), 2.0, Some(0.0), true));
            work.push(99);
            panic!("interrupt before commit");
        }));
        assert!(interrupted.is_err());
        assert_eq!(cache.stored().unwrap(), &[1, 2]);
        assert_eq!(cache.stored().unwrap().as_ptr(), original);
        assert!(cache.has_been_invalidated);
        assert!(std::panic::catch_unwind(|| cache.value()).is_err());
        work.clear();
        work.extend([1, 2]);
        assert!(cache.needs_refresh(&(), 2.0, Some(0.0), true));
        assert!(!cache.store_reusing((), 2.0, 0.0, &mut work).value_changed);
        assert_eq!(cache.generation, 1);
        assert_eq!(cache.calculated_at, Some(2.0));
        assert_eq!(work.as_ptr(), original);
    }

    #[test]
    fn non_clone_elements_are_compared_once_and_never_copied() {
        struct Counted<'a>(&'a std::cell::Cell<usize>);
        impl PartialEq for Counted<'_> {
            fn eq(&self, _: &Self) -> bool { self.0.set(self.0.get() + 1); true }
        }
        let comparisons = std::cell::Cell::new(0);
        let mut cache = Cache::default();
        let mut work = vec![Counted(&comparisons), Counted(&comparisons)];
        cache.store_reusing((), 1.0, 0.0, &mut work);
        assert_eq!(comparisons.get(), 0);
        work.extend([Counted(&comparisons), Counted(&comparisons)]);
        assert!(!cache.store_reusing((), 2.0, 0.0, &mut work).value_changed);
        assert_eq!(comparisons.get(), 2);
    }
}

#[cfg(test)]
mod pair_reuse_tests {
    use super::*;
    #[test]
    fn pair_swap_preserves_metadata_generations_and_both_displaced_allocations() {
        let mut cached: Cache<u8, (Vec<u32>, Vec<u32>, u32)> = Cache::default();
        let mut reference = Cache::default();
        let mut work = (Vec::with_capacity(8), Vec::with_capacity(4), 0);
        for (epoch, key, star) in [(1.0, 1, 3), (2.0, 2, 3), (3.0, 2, 4)] {
            let values = (vec![star, 7], vec![9], 10);
            assert!(cached.needs_refresh(&key, epoch, Some(0.0), true));
            reference.needs_refresh(&key, epoch, Some(0.0), true);
            work.0.extend_from_slice(&values.0); work.1.extend_from_slice(&values.1); work.2 = values.2;
            let incoming = (work.0.as_ptr(), work.1.as_ptr());
            let displaced = cached.stored().map(|old| (old.0.as_ptr(), old.1.as_ptr()));
            assert_eq!(cached.store_reusing_pair(key, epoch, 0.0, &mut work), reference.store(key, epoch, 0.0, values));
            assert_eq!(cached, reference);
            assert_eq!((cached.value().0.as_ptr(), cached.value().1.as_ptr()), incoming);
            if let Some(displaced) = displaced { assert_eq!((work.0.as_ptr(), work.1.as_ptr()), displaced); }
            assert!(work.0.is_empty() && work.1.is_empty());
        }
        assert_eq!(cached.generation, 2);
        cached.invalidate();
        work.0.push(99); // partial replacement is separate from the invalid previous result
        assert_eq!(cached.stored().unwrap().0, [4, 7]);
        assert!(std::panic::catch_unwind(|| cached.value()).is_err());
    }
}
