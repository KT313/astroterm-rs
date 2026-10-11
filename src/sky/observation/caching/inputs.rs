//! Validate provenance before any cached correction can read prepared model results.
use crate::{state::{ObservationCache, StellarResults, PreparedBodies}, model::{ObserverState, SkyCatalog}};
use std::sync::Arc;
pub(in crate::sky::observation) fn prepare_inputs(storage: &mut ObservationCache, stars: StellarResults<'_>, bodies: PreparedBodies<'_>, observer: &ObserverState, catalog: &Arc<SkyCatalog>) {
    assert!(std::sync::Arc::ptr_eq(stars.selection.catalog, catalog), "observed catalog does not match stellar inputs");
    assert!(bodies.cache.key().is_some_and(|key| key.0 == *observer), "body inputs do not match observer");
    assert_eq!(stars.selection.epoch, observer.time.tt, "stellar and observer times differ");
    let sources = (stars.selection.key.0, stars.identity, bodies.identity);
    if storage.sources != Some(sources) || storage.catalog.as_ref().is_none_or(|previous| !Arc::ptr_eq(previous, catalog)) {
        *storage = ObservationCache::new(storage.config.clone());
        storage.catalog = Some(catalog.clone());
        storage.sources = Some(sources);
        storage.regions.resize_with(crate::constants::SIMULATION_REGION_COUNT, Default::default);
    }
}
