//! Prepare immutable stellar classifications once; no observer state is touched.
use std::sync::Arc;
use crate::state::StellarSimulationState;
use crate::model::SkyCatalog;
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain};
pub fn prepare_stellar_catalog(storage: &mut StellarSimulationState, catalog: Arc<SkyCatalog>, start_tt: f64, times: &mut StepTimes) {
    *storage = StellarSimulationState::new(storage.config.clone());
    let classes: Vec<_> = times.measure("Stellar classifications", || {
        let trajectories = catalog.stars.borrow_trajectory_fields();
        (0..catalog.stars.len())
            .map(|i| trajectories.motion(i).classify())
            .collect()
    });
    {
        times.record_borrow(BufferId::CatalogTrajectories, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_build(BufferId::CatalogClassifications, || BufferShape::vector(&classes, IndexDomain::Catalog));
    }
    times.describe("Stellar classifications", || {
        format!(
            "stars={}; stationary={}; moving with distance={}; classification bytes={}",
            classes.len(),
            classes.iter().filter(|c| c.is_stationary()).count(),
            classes.iter().filter(|c| c.has_variable_brightness()).count(),
            classes.len() * std::mem::size_of::<crate::astro::models::stars::StellarClass>()
        )
    });
    times.measure("Stellar region initialization", || storage.initialize_regions(start_tt));
    times.record_build(BufferId::StellarSamples, || BufferShape::vector(&storage.regions.entries, IndexDomain::Regions));
    storage.prepared_classes = Some(classes);
    storage.catalog = Some(catalog);
}
