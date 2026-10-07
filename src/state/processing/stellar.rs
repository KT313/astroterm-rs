//! Intrinsic catalog-star samples and bounded scratch. Camera and observer state never enter this owner.
use std::{sync::Arc, collections::HashMap};
use crate::astro::{Vector3, models::stars::StellarSample};
use crate::cache::{Cache, CacheConfig};
use crate::model::SkyCatalog;
use crate::state::WorkingCache;
pub(crate) type MotionCache = Cache<(super::StageId, u64), (Vec<(Vector3, f64)>, usize)>;
#[derive(Default)]
pub struct StellarSimulationState {
    pub(crate) requested_epoch: Option<f64>,
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,
    pub(crate) prepared_classes: Option<Vec<crate::astro::models::stars::StellarClass>>, // immutable catalog-row classifications; absent means classify on demand
    pub(crate) stellar: HashMap<usize, Cache<(), StellarSample>>,       // intrinsic J2000 direction/magnitude per catalog row; no eviction
    pub(crate) stellar_scratch: Vec<crate::model::StellarWork>, // at most 1024 live records; clear after refresh, retain capacity
    pub(crate) stellar_stats: crate::cache::CacheStats,
    pub(crate) motion: MotionCache,                                    // J2000 unit directions/current magnitudes in working order, plus singular count
}
/// Only stellar sample/cache outputs can change; membership and catalog classifications stay borrowed.
pub(crate) struct StellarMotionBuffers<'a> {
    pub config: &'a CacheConfig,
    pub key: (super::StageId, u64),
    pub working: &'a WorkingCache,
    pub prepared_classes: Option<&'a [crate::astro::models::stars::StellarClass]>,
    pub stellar: &'a mut HashMap<usize, Cache<(), StellarSample>>,
    pub scratch: &'a mut Vec<crate::model::StellarWork>,
    pub stats: &'a mut crate::cache::CacheStats,
    pub motion: &'a mut MotionCache,
}

impl StellarSimulationState {
    pub fn new(config: CacheConfig) -> Self { Self { config, ..Self::default() } }
    pub(crate) fn borrow_stellar_motion<'a>(&'a mut self, selection: crate::state::SelectedStars<'a>) -> StellarMotionBuffers<'a> {
        StellarMotionBuffers {
            config: &self.config, working: selection.working, key: selection.key, prepared_classes: self.prepared_classes.as_deref(),
            stellar: &mut self.stellar, scratch: &mut self.stellar_scratch, stats: &mut self.stellar_stats,
            motion: &mut self.motion,
        }
    }
    pub fn stellar_report(&self, index: usize) -> Option<crate::cache::CacheReport> {
        self.stellar.get(&index).map(|c| c.report("Stellar state"))
    }
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> { vec![self.motion.report("Stellar motion")] }
    pub fn stats(&self) -> crate::cache::CacheStats { super::sum_stats([self.motion.stats, self.stellar_stats]) }
    pub fn results<'a>(&'a self, selection: crate::state::SelectedStars<'a>) -> StellarResults<'a> {
        assert!(self.catalog.as_ref().is_some_and(|catalog| Arc::ptr_eq(catalog, selection.catalog)), "stellar catalog does not match selection");
        assert_eq!(self.motion.key(), Some(&selection.key), "stellar results do not match selection");
        assert_eq!(self.requested_epoch, Some(selection.epoch), "stellar results do not match selection time");
        self.motion.value(); // reject an invalidated result before publishing a view
        StellarResults { selection, motion: &self.motion, identity: self.identity }
    }
}
/// Intrinsic outputs and their matching selection remain read-only throughout observation.
#[derive(Clone, Copy)]
pub struct StellarResults<'a> {
    pub(crate) selection: crate::state::SelectedStars<'a>,
    pub(crate) motion: &'a MotionCache,
    pub(crate) identity: super::StageId,
}
impl StellarResults<'_> {
    pub fn samples(&self) -> &[(Vector3, f64)] { &self.motion.value().0 }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StellarSimulationState { config, catalog, prepared_classes, stellar, stellar_scratch, motion });
