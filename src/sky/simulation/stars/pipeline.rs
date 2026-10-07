//! Update selected stars without access to the viewer, camera, or observed output.
use crate::state::{StellarSimulationState, SelectedStars};
use crate::timing::StepTimes;
use std::sync::Arc;

pub fn simulate_stars(storage: &mut StellarSimulationState, selection: SelectedStars<'_>, epoch: f64, times: &mut StepTimes) {
    assert_eq!(selection.epoch, epoch, "selection and stellar simulation times differ");
    if storage.catalog.as_ref().is_none_or(|previous| !Arc::ptr_eq(previous, selection.catalog)) {
        *storage = StellarSimulationState::new(storage.config.clone());
        storage.catalog = Some(selection.catalog.clone());
    }
    let previous = times.trace().map(|_| storage.reports());
    times.measure_steps("Stellar motion", |times| {
        super::processing::update_stellar_motion(storage.borrow_stellar_motion(selection), selection.catalog.stars.borrow_stellar_fields(), epoch, times);
    });
    storage.requested_epoch = Some(epoch);
    super::diagnostics::describe_results(storage, selection, times);
    crate::sky::describe_cache_reports(previous, || storage.reports(), times);
}
