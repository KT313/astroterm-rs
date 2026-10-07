use crate::{state::{StellarSimulationState, SelectedStars}, timing::StepTimes};
pub(super) fn describe_results(storage: &StellarSimulationState, selection: SelectedStars<'_>, times: &mut StepTimes) {
    times.measure_diagnostics(|times| {
        times.describe("Stellar motion", || format!("input working stars={}; output directions/magnitudes={}; singular fallbacks={}; stellar sample cache totals: hits={} refreshes={} bypasses={}; stars use catalog propagation, no per-star light-time solve", selection.rows().len(), storage.motion.value().0.len(), storage.motion.value().1, storage.stellar_stats.hits, storage.stellar_stats.refreshes, storage.stellar_stats.bypasses));
    });
}
