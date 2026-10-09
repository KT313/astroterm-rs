//! Intrinsic samples owned once per region. Camera and observer state never enter this owner.
use crate::constants::SIMULATION_REGION_COUNT;
use std::sync::Arc;
use crate::astro::models::stars::StellarSample;
use crate::cache::{Cache, CacheConfig, CacheStats};
use crate::model::SkyCatalog;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StellarRequest {
    pub selection_key: (super::StageId, u64),
    pub selection_revision: u64,
    pub epoch: f64,
    pub regional_revision: u64,
}
crate::rows::row_columns!(StellarRequest { selection_key, selection_revision, epoch, regional_revision });

pub(crate) type StellarPublication = (super::StageId, StellarRequest); // owning simulation plus its complete selection/time request

/// Original regional owner; each populated entry contains its entire catalog range in the same order.
#[derive(Default)]
pub(crate) struct StellarRegions {
    pub entries: Vec<Cache<(), Vec<StellarSample>>>,
}

#[derive(Default)]
pub struct StellarSimulationState {
    pub(crate) last_request: Option<StellarRequest>, // published only after every requested region is complete
    pub(crate) selected_fallback_count: usize,
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,
    pub(crate) prepared_classes: Option<Vec<crate::astro::models::stars::StellarClass>>,
    pub(crate) regions: StellarRegions,
    pub(crate) refresh_regions: Vec<usize>,
    pub(crate) stellar_scratch: Vec<crate::model::StellarWork>, // bounded numeric scratch; no per-star cache metadata
    pub(crate) region_stats: CacheStats,
    pub(crate) region_results_generation: u64,
    pub(crate) region_output_work: Vec<StellarSample>, // one replacement region; receives the displaced allocation at commit
}
/// Only regional outputs can change; requested membership, ranges and catalog classifications stay borrowed.
pub(crate) struct StellarMotionBuffers<'a> {
    pub config: &'a CacheConfig,
    pub requested_regions: &'a [usize],
    pub offsets: &'a [usize],
    pub prepared_classes: Option<&'a [crate::astro::models::stars::StellarClass]>,
    pub regions: &'a mut StellarRegions,
    pub refresh_regions: &'a mut Vec<usize>,
    pub scratch: &'a mut Vec<crate::model::StellarWork>,
    pub stats: &'a mut CacheStats,
    pub generation: &'a mut u64,
    pub output_work: &'a mut Vec<StellarSample>,
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
            config: &self.config,
            requested_regions: selection.regions, offsets: &selection.catalog.grid.offsets,
            prepared_classes: self.prepared_classes.as_deref(), regions: &mut self.regions,
            refresh_regions: &mut self.refresh_regions, scratch: &mut self.stellar_scratch,
            stats: &mut self.region_stats, generation: &mut self.region_results_generation, output_work: &mut self.region_output_work,
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
        self.last_request = None; // incomplete regional results cannot be published
    }
    pub fn stats(&self) -> CacheStats { self.region_stats }
    pub(crate) fn request_key(&self, selection: crate::state::SelectedStars<'_>) -> StellarRequest {
        StellarRequest { selection_key: selection.key, selection_revision: selection.request_revision(), epoch: selection.epoch, regional_revision: self.region_results_generation }
    }
    pub fn results<'a>(&'a self, selection: crate::state::SelectedStars<'a>) -> StellarResults<'a> {
        assert!(self.catalog.as_ref().is_some_and(|catalog| Arc::ptr_eq(catalog, selection.catalog)), "stellar catalog does not match selection");
        assert_eq!(self.last_request, Some(self.request_key(selection)), "stellar results do not match selection request/time");
        StellarResults { selection, regions: &self.regions, identity: self.identity,
            fallback_count: self.selected_fallback_count, revision: self.region_results_generation }
    }
}
/// Outputs match this request, but their calculation epochs belong to independently held regions.
#[derive(Clone, Copy)]
pub struct StellarResults<'a> {
    pub(crate) selection: crate::state::SelectedStars<'a>,
    pub(crate) regions: &'a StellarRegions,
    pub(crate) identity: super::StageId,
    fallback_count: usize,
    revision: u64,
}
impl StellarResults<'_> {
    pub(crate) fn publication_key(&self) -> StellarPublication {
        (self.identity, StellarRequest { selection_key: self.selection.key, selection_revision: self.selection.request_revision(), epoch: self.selection.epoch, regional_revision: self.revision })
    }
    pub fn selected_count(&self) -> usize { self.selection.rows().len() }
    pub fn fallback_count(&self) -> usize { self.fallback_count }
    /// Borrow original samples in working-row order; each regional slice is validated once.
    pub fn selected_samples(&self) -> impl Iterator<Item = (usize, &StellarSample)> {
        borrow_selected_samples(self.selection, self.regions)
    }
    /// Only requested regions have been checked for freshness for this frame.
    pub fn region_samples(&self, region: usize) -> &[StellarSample] {
        assert!(self.selection.regions.binary_search(&region).is_ok(), "region was not requested");
        self.regions.entries[region].value()
    }
    pub fn region_generation(&self, region: usize) -> u64 {
        self.region_samples(region); // check validity without visiting any individual sample
        self.regions.entries[region].generation
    }
}
/// Walk sorted working rows and requested regions together without collecting or looking up each star.
fn borrow_selected_samples<'a>(selection: crate::state::SelectedStars<'a>, regions: &'a StellarRegions) -> impl Iterator<Item = (usize, &'a StellarSample)> {
    let mut rows = selection.working.value().as_slice();
    selection.regions.iter().flat_map(move |&region| {
        let start = selection.catalog.grid.offsets[region];
        let end = selection.catalog.grid.offsets[region + 1];
        let samples = regions.entries[region].value();
        assert_eq!(samples.len(), end - start, "region must be completely populated");
        let (selected, remaining) = rows.split_at(rows.partition_point(|row| row.source_index < end));
        rows = remaining;
        selected.iter().map(move |row| (row.source_index, &samples[row.source_index - start]))
    })
}
impl StellarSimulationState {
    pub(crate) fn count_selected_fallbacks(&self, selection: crate::state::SelectedStars<'_>) -> usize {
        borrow_selected_samples(selection, &self.regions).filter(|(_, sample)| sample.used_singular_fallback).count()
    }
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
crate::cache::report_fields!(StellarSimulationState { config, catalog, prepared_classes, regions, refresh_regions, stellar_scratch, region_output_work });
