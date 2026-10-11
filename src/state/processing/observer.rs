//! Reception geometry, emission epochs and final body states; independent of the star catalog.
use crate::cache::{Cache, CacheConfig};
use crate::model::{ObserverKey, ObserverState, ObservationBodyKey, BodySamples};
#[derive(Default)]
pub struct ObserverPreparationCache {
    pub(crate) solar_request: Option<((super::StageId, u64), [u64; 3])>,
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
    pub(crate) fn source_id(&self) -> u64 { self.identity.value() }
    pub(crate) fn permits_solar_reuse(&self) -> bool {
        use crate::cache::Group;
        [Group::ObserverState, Group::SolarSystemObservation].into_iter().all(|group| self.config.allows(group))
    }
    pub(crate) fn invalidate_solar_request(&mut self) {
        self.solar_request = None;
        self.observer.invalidate();
        self.light_time.invalidate();
        self.bodies.invalidate();
    }
    fn result_versions(&self) -> [u64; 3] { [self.observer.generation, self.light_time.generation, self.bodies.generation] }
    pub(crate) fn complete_solar_request(&mut self, token: (super::StageId, u64)) {
        self.solar_request = Some((token, self.result_versions())); // bind completion to the actual companion results, not just this owner
    }
    pub(crate) fn completed_observer(&self, token: (super::StageId, u64)) -> Option<ObserverState> {
        if self.solar_request != Some((token, self.result_versions())) || self.observer.has_been_invalidated || self.light_time.has_been_invalidated || self.bodies.has_been_invalidated { return None; }
        let observer = *self.light_time.stored()?;
        if self.bodies.key()?.0 != observer || self.bodies.stored().is_none() { return None; }
        Some(observer)
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
