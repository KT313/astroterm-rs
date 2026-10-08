//! Describe the completed selection while its timing scope is active.
use crate::{state::StarSelectionCache, model::SkyCatalog, timing::StepTimes};
pub(in crate::sky::selection) fn describe_selection(storage: &StarSelectionCache, catalog: &SkyCatalog, threshold: f64, times: &mut StepTimes) {
    times.measure_diagnostics(|times| {
        let total = catalog.stars.len();
        let (cells, regional) = crate::sky::count_region_stars(&catalog.grid, storage.region.value(), total);
        times.describe("Region filtering", || format!("input stars={total}; selected spatial cells={cells}/{}; constellation region requested=true; retained by catalog region={regional}; rejected region={}; brute-force={}", crate::constants::CELL_COUNT, total - regional, storage.candidates.value().1.brute_force));
        times.describe("Brightness bounds", || format!("input regional stars={regional}; rejected interval magnitude bound > {threshold}={}; output candidates={}; brute-force bypass={}", regional - storage.candidates.value().0.len(), storage.candidates.value().0.len(), storage.candidates.value().1.brute_force));
        times.describe("Candidate validation", || {
            let candidates = &storage.candidates.value().0;
            let invalid = candidates.iter().filter(|&&i| i >= total).count();
            let removed = candidates.len() - storage.selected.value().len();
            format!("input candidates={}; rejected invalid index={invalid}; then rejected bound > {threshold}={}; output candidates={}; outside interval uses all stars", candidates.len(), removed - invalid, storage.selected.value().len())
        });
        let working = storage.working.value();
        times.describe("Constellation endpoints", || format!("input selected={}; endpoint union={}; added endpoint-only={}; output working stars={}; endpoints included even when constellation drawing is disabled", storage.selected.value().len(), catalog.endpoint_indices().len(), working.len() - storage.selected.value().len(), working.len()));
    });
}
