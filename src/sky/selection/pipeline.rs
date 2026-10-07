//! Region, brightness bounds, validation and endpoint merge in execution order.
use crate::state::StarSelectionCache;
use crate::model::{SkyCatalog, ObserverState, SkyRegion};
use crate::timing::StepTimes;
use std::sync::Arc;
use super::caching::*;

pub fn select_cached_stars(storage: &mut StarSelectionCache, catalog: &Arc<SkyCatalog>, observer: &ObserverState,
    threshold: f64, refraction: bool, region: SkyRegion, times: &mut StepTimes) {
    if storage.catalog.as_ref().is_none_or(|previous| !Arc::ptr_eq(previous, catalog)) {
        *storage = StarSelectionCache::new(storage.config.clone());
        storage.catalog = Some(catalog.clone());
    }
    let previous = times.trace().map(|_| storage.reports());
    let epoch = observer.time.tt;
    update_region_filtering(&mut storage.region, &storage.config, epoch, refraction, region, observer, &catalog.grid, times);
    update_brightness_bounds(&mut storage.candidates, &storage.region, &storage.config, epoch, threshold, catalog, times);
    update_candidate_validation(&mut storage.selected, &storage.candidates, &storage.config, epoch, threshold, catalog, times);
    update_constellation_endpoints(&mut storage.working, &storage.selected, &storage.config, epoch, catalog.endpoint_indices(), times);
    storage.requested_epoch = Some(epoch);
    describe_selection(storage, catalog, threshold, times);
    crate::sky::describe_cache_reports(previous, || storage.reports(), times);
}
