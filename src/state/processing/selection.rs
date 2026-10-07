//! Conservative catalog-row selection, independent of intrinsic model samples and corrections.
use std::sync::Arc;
use crate::cache::{Cache, CacheConfig};
use crate::model::{SkyCatalog, ObserverState, SelectedStar};
pub(crate) type RegionCache = Cache<(crate::model::SkyRegion, ObserverState, bool), crate::model::SelectedRegion>;
pub(crate) type CandidateCache = Cache<(u64, f64), (Vec<usize>, crate::model::SelectionStats)>;
pub(crate) type SelectedCache = Cache<(u64, bool), Vec<usize>>;
pub(crate) type WorkingCache = Cache<u64, Vec<SelectedStar>>;
#[derive(Default)]
pub struct StarSelectionCache {
    pub(crate) requested_epoch: Option<f64>,
    pub(crate) identity: super::StageId,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,
    pub(crate) region: RegionCache,
    pub(crate) candidates: CandidateCache,
    pub(crate) selected: SelectedCache,
    pub(crate) working: WorkingCache,
}
impl StarSelectionCache {
    pub fn new(config: CacheConfig) -> Self { Self { config, ..Self::default() } }
    pub fn invalidate_view(&mut self) { self.region.invalidate(); }
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![self.region.report("Region filtering"), self.candidates.report("Brightness bounds"), self.selected.report("Candidate validation"), self.working.report("Constellation endpoints")]
    }
    pub fn stats(&self) -> crate::cache::CacheStats { super::sum_stats([self.region.stats, self.candidates.stats, self.selected.stats, self.working.stats]) }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarSelectionCache { config, catalog, region, candidates, selected, working });

/// Immutable working rows with their catalog and source generation. No row data is copied.
#[derive(Clone, Copy)]
pub struct SelectedStars<'a> {
    pub(crate) catalog: &'a Arc<SkyCatalog>,
    pub(crate) working: &'a WorkingCache,
    pub(crate) key: (super::StageId, u64),
    pub(crate) epoch: f64,
    pub(crate) statistics: crate::model::SelectionStats,
}
impl StarSelectionCache {
    pub fn stars(&self) -> SelectedStars<'_> {
        SelectedStars { catalog: self.catalog.as_ref().expect("selection prepared"), working: &self.working,
            key: (self.identity, self.working.generation), epoch: self.requested_epoch.expect("selection prepared"), statistics: self.candidates.value().1 }
    }
}
impl SelectedStars<'_> {
    pub fn rows(&self) -> &[SelectedStar] { self.working.value() }
}
