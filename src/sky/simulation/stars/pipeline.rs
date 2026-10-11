//! Refresh requested intrinsic regions, then publish borrowed results without viewer/camera access.
use crate::state::{StellarSimulationState, SelectedStars};
use crate::timing::StepTimes;
use std::sync::Arc;

pub fn simulate_stars(storage: &mut StellarSimulationState, selection: SelectedStars<'_>, epoch: f64, times: &mut StepTimes) {
    assert_eq!(selection.epoch, epoch, "selection and stellar simulation times differ");
    if storage.catalog.as_ref().is_none_or(|previous| !Arc::ptr_eq(previous, selection.catalog)) {
        super::preparation::prepare_stellar_catalog(storage, selection.catalog.clone(), epoch, times);
    }
    let previous = storage.last_request.take(); // failed preparation cannot leave an older request published
    times.measure_steps("Stellar motion", |times| {
        super::processing::update_stellar_motion(storage.borrow_stellar_motion(selection), selection.catalog.stars.borrow_stellar_fields(), epoch, times);
        let request = storage.request_key(selection);
        if previous.is_none_or(|previous| previous.selection_key != request.selection_key || previous.regional_revision != request.regional_revision) {
            storage.selected_fallback_count = times.measure("Selected fallback count", || storage.count_selected_fallbacks(selection));
        }
        storage.last_request = Some(request); // all requested regions are complete; publish only scalar metadata
    });
    super::diagnostics::describe_results(storage, selection, times);
}
