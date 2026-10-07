//! Reception geometry, emission epochs and final body states; independent of the star catalog.
use crate::cache::{Cache, CacheConfig};
use crate::model::{ObserverKey, ObserverState, ObservationBodyKey, BodySamples};
#[derive(Default)]
pub struct ObserverPreparationCache {
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) observer: Cache<ObserverKey, ObserverState>,
    pub(crate) light_time: Cache<(ObserverState, [u64; 3]), ObserverState>,
    pub(crate) bodies: Cache<ObservationBodyKey, BodySamples>,
}
/// Observer preparation can mutate only its reception cache, never selection/correction buffers.
pub(crate) struct ObserverBuffers<'a> {
    pub config: &'a CacheConfig,
    pub observer: &'a mut Cache<ObserverKey, ObserverState>,
}

/// Emission preparation can mutate only its result cache; simulation owns the actual body samples.
pub(crate) struct LightTimeBuffers<'a> {
    pub config: &'a CacheConfig,
    pub light_time: &'a mut Cache<(ObserverState, [u64; 3]), ObserverState>,
}
impl ObserverPreparationCache {
    pub fn new(config: CacheConfig) -> Self { Self { config, ..Self::default() } }
    pub(crate) fn borrow_observer(&mut self) -> ObserverBuffers<'_> {
        ObserverBuffers { config: &self.config, observer: &mut self.observer }
    }
    pub(crate) fn borrow_light_time(&mut self) -> LightTimeBuffers<'_> {
        LightTimeBuffers { config: &self.config, light_time: &mut self.light_time }
    }
    pub fn observer_report(&self) -> crate::cache::CacheReport { self.observer.report("Observer geometry") }
    pub fn light_time_report(&self) -> crate::cache::CacheReport { self.light_time.report("Light-time sampling") }
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![self.observer_report(), self.light_time_report(), self.bodies.report("Body sampling")]
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        super::sum_stats([self.observer.stats, self.light_time.stats, self.bodies.stats])
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(ObserverPreparationCache { config, observer, light_time, bodies });

#[derive(Clone, Copy)]
pub struct PreparedBodies<'a> {
    pub(crate) cache: &'a Cache<ObservationBodyKey, BodySamples>,
    pub(crate) identity: super::StageId,
}
impl ObserverPreparationCache {
    pub fn bodies(&self, observer: &ObserverState) -> PreparedBodies<'_> {
        assert!(self.bodies.key().is_some_and(|key| key.0 == *observer), "body samples do not match observer");
        self.bodies.value();
        PreparedBodies { cache: &self.bodies, identity: self.identity }
    }
}
