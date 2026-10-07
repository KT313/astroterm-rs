//! Intrinsic samples owned once per region. Camera and observer state never enter this owner.
use std::sync::Arc;
use crate::astro::{Vector3, models::stars::StellarSample};
use crate::cache::{Cache, CacheConfig, CacheStats};
use crate::model::{SkyCatalog, SIMULATION_REGION_COUNT};
use crate::state::WorkingCache;
pub(crate) type MotionCache = Cache<(super::StageId, u64, u64), (Vec<(Vector3, f64)>, usize)>;

/// Original regional owner; each populated entry contains its entire catalog range in the same order.
#[derive(Default)]
pub(crate) struct StellarRegions {
    pub entries: Vec<Cache<(), Vec<StellarSample>>>,
}

#[derive(Default)]
pub struct StellarSimulationState {
    pub(crate) requested_epoch: Option<f64>,
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,
    pub(crate) prepared_classes: Option<Vec<crate::astro::models::stars::StellarClass>>,
    pub(crate) regions: StellarRegions,
    pub(crate) refresh_regions: Vec<usize>,
    pub(crate) stellar_scratch: Vec<crate::model::StellarWork>, // bounded numeric scratch; no per-star cache metadata
    pub(crate) region_stats: CacheStats,
    pub(crate) region_results_generation: u64,
    pub(crate) motion: MotionCache, // selected-row order only; region caches own the reusable samples
}
/// Only regional outputs can change; requested membership, ranges and catalog classifications stay borrowed.
pub(crate) struct StellarMotionBuffers<'a> {
    pub config: &'a CacheConfig,
    pub key: (super::StageId, u64),
    pub working: &'a WorkingCache,
    pub requested_regions: &'a [usize],
    pub offsets: &'a [usize],
    pub prepared_classes: Option<&'a [crate::astro::models::stars::StellarClass]>,
    pub regions: &'a mut StellarRegions,
    pub refresh_regions: &'a mut Vec<usize>,
    pub scratch: &'a mut Vec<crate::model::StellarWork>,
    pub stats: &'a mut CacheStats,
    pub generation: &'a mut u64,
    pub motion: &'a mut MotionCache,
}

impl StellarSimulationState {
    pub fn new(config: CacheConfig) -> Self { Self { config, ..Self::default() } }
    pub(crate) fn initialize_regions(&mut self, start_tt: f64) {
        self.regions.entries = (0..SIMULATION_REGION_COUNT).map(|_| {
            let mut cache = Cache::default();
            cache.calculated_at = Some(start_tt);
            cache
        }).collect();
    }
    pub(crate) fn borrow_stellar_motion<'a>(&'a mut self, selection: crate::state::SelectedStars<'a>) -> StellarMotionBuffers<'a> {
        StellarMotionBuffers {
            config: &self.config, working: selection.working, key: selection.key,
            requested_regions: selection.regions, offsets: &selection.catalog.grid.offsets,
            prepared_classes: self.prepared_classes.as_deref(), regions: &mut self.regions,
            refresh_regions: &mut self.refresh_regions, scratch: &mut self.stellar_scratch,
            stats: &mut self.region_stats, generation: &mut self.region_results_generation, motion: &mut self.motion,
        }
    }
    pub fn region_report(&self, region: usize) -> Option<crate::cache::CacheReport> {
        self.regions.entries.get(region).map(|c| c.report("Stellar region"))
    }
    /// Read a completed region; an expired sample is inspectable at its reported calculation epoch.
    pub fn region_samples(&self, region: usize) -> Option<&[StellarSample]> {
        let entry = self.regions.entries.get(region)?;
        if entry.has_been_invalidated { return None; }
        entry.stored().map(Vec::as_slice)
    }
    pub fn invalidate_region(&mut self, region: usize) {
        self.regions.entries.get_mut(region).expect("known simulation region").invalidate();
        self.motion.invalidate(); // a previously published working list may depend on this region
    }
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> { vec![self.motion.report("Stellar motion")] }
    pub fn stats(&self) -> CacheStats { super::sum_stats([self.motion.stats, self.region_stats]) }
    pub fn results<'a>(&'a self, selection: crate::state::SelectedStars<'a>) -> StellarResults<'a> {
        assert!(self.catalog.as_ref().is_some_and(|catalog| Arc::ptr_eq(catalog, selection.catalog)), "stellar catalog does not match selection");
        let key = (selection.key.0, selection.key.1, self.region_results_generation);
        assert_eq!(self.motion.key(), Some(&key), "stellar results do not match selection");
        assert_eq!(self.requested_epoch, Some(selection.epoch), "stellar results do not match selection time");
        self.motion.value(); // reject an invalidated result before publishing a view
        StellarResults { selection, motion: &self.motion, identity: self.identity }
    }
}
/// Outputs match this request, but their calculation epochs belong to independently held regions.
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
impl crate::cache::ReportBuffers for StellarRegions {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::Quality;
        sink.payload(self.entries.len(), self.entries.capacity(), std::mem::size_of::<Cache<(), Vec<StellarSample>>>(), Quality::ExactPayload, "region slots; samples aggregated separately");
        if sink.enter("samples", 0) {
            let totals = self.entries.iter().filter_map(Cache::stored).try_fold((0_usize, 0_usize), |(len, capacity), samples| {
                Some((len.checked_add(samples.len())?, capacity.checked_add(samples.capacity())?))
            });
            if let Some((len, capacity)) = totals {
                sink.payload(len, capacity, std::mem::size_of::<StellarSample>(), Quality::ExactPayload, "sum of disjoint regional sample allocations; no per-region traversal rows");
            } else { sink.unknown("regional sample payload overflow"); }
            sink.leave();
        }
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StellarSimulationState { config, catalog, prepared_classes, regions, refresh_regions, stellar_scratch, motion });
