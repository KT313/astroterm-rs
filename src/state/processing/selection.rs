//! Conservative catalog-row selection, independent of intrinsic model samples and corrections.
use std::sync::Arc;
use crate::cache::{Cache, CacheConfig};
use crate::model::{SkyCatalog, ObserverState, SelectedStar};
pub(crate) type RegionCache = Cache<(crate::model::SkyRegion, ObserverState, bool), crate::model::SelectedRegion>;
pub(crate) type WorkingCache = Cache<u64, Vec<SelectedStar>>;
#[derive(Default)]
pub struct StarSelectionCache {
    pub(crate) candidate_region_stats: crate::cache::CacheStats,
    pub(crate) selected_region_stats: crate::cache::CacheStats,
    pub(crate) requested_epoch: Option<f64>,
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,
    pub(crate) region: RegionCache,
    pub(crate) statistics: crate::model::SelectionStats,
    pub(crate) requested_sources: Vec<(usize, u64)>, // ordered region IDs/versions, never individual star indices
    pub(crate) selection_revision: u64,              // distinguishes requests even when their working rows are equal
    pub(crate) working: WorkingCache,
    pub(crate) region_candidates: Vec<Cache<(f64, bool), (usize, usize)>>, // each region retains its qualifying catalog prefix
    pub(crate) region_selected: Vec<Cache<u64, (usize, usize)>>,          // validated prefix; versions survive changes in requested regions
}
impl StarSelectionCache {
    pub fn new(config: CacheConfig) -> Self { Self { config, ..Self::default() } }
    pub fn invalidate_view(&mut self) { self.region.invalidate(); self.requested_epoch = None; }
    pub fn invalidate_region(&mut self, region: usize) {
        self.region_candidates.get_mut(region).expect("known selection region").invalidate();
        self.region_selected.get_mut(region).expect("known selection region").invalidate();
        self.requested_epoch = None;
        self.working.invalidate();
    }
    pub fn brightness_region_report(&self, region: usize) -> Option<crate::cache::CacheReport> {
        self.region_candidates.get(region).map(|entry| entry.report("Regional brightness bounds"))
    }
    pub fn validation_region_report(&self, region: usize) -> Option<crate::cache::CacheReport> {
        self.region_selected.get(region).map(|entry| entry.report("Regional candidate validation"))
    }
    /// These are the two whole-request caches; regional decisions have their own reports and counters.
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![self.region.report("Region filtering"), self.working.report("Constellation endpoints")]
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        super::sum_stats([self.region.stats, self.working.stats, self.candidate_region_stats, self.selected_region_stats])
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarSelectionCache { config, catalog, region, requested_sources, statistics, working, region_candidates, region_selected });

/// Immutable working rows with their catalog and source generation. No row data is copied.
#[derive(Clone, Copy)]
pub struct SelectedStars<'a> {
    pub(crate) catalog: &'a Arc<SkyCatalog>,
    pub(crate) working: &'a WorkingCache,
    pub(crate) key: (super::StageId, u64),
    pub(crate) epoch: f64,
    pub(crate) statistics: crate::model::SelectionStats,
    pub(crate) regions: &'a [usize],
    request_revision: u64,
    regional_selection: &'a [Cache<u64, (usize, usize)>],
}
impl StarSelectionCache {
    pub fn stars(&self) -> SelectedStars<'_> {
        self.working.value(); // only completed working rows can be published
        assert_eq!(self.working.key(), Some(&self.selection_revision), "working rows do not match the selected regions");
        SelectedStars { catalog: self.catalog.as_ref().expect("selection prepared"), working: &self.working,
            key: (self.identity, self.working.generation), epoch: self.requested_epoch.expect("selection prepared"), statistics: self.statistics, request_revision: self.selection_revision, regions: &self.region.value().cells, regional_selection: &self.region_selected }
    }
}
impl SelectedStars<'_> {
    pub fn rows(&self) -> &[SelectedStar] { self.working.value() }
    pub fn regions(&self) -> &[usize] { self.regions }
    /// Iterate original cached prefixes: region ID, catalog start/end (exclusive), result generation.
    pub fn ranges(&self) -> impl ExactSizeIterator<Item = (usize, usize, usize, u64)> + '_ {
        self.regions.iter().map(|&region| {
            let entry = &self.regional_selection[region];
            let &(start, end) = entry.value();
            (region, start, end, entry.generation)
        })
    }
    /// Request identity changes independently from the value-based working-row generation.
    pub fn request_revision(&self) -> u64 { self.request_revision }
    /// Region-local candidate version; other regions entering or leaving the view do not change it.
    pub fn region_generation(&self, region: usize) -> u64 {
        assert!(self.regions.binary_search(&region).is_ok(), "region was not requested");
        self.regional_selection[region].value(); // only publish completed regional validation
        self.regional_selection[region].generation
    }
}
