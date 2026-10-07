use crate::{state::{StellarSimulationState, SelectedStars}, timing::StepTimes};
pub(super) fn describe_results(storage: &StellarSimulationState, selection: SelectedStars<'_>, times: &mut StepTimes) {
    times.describe("Stellar motion", || format!("input working stars={}; output directions/magnitudes={}; singular fallbacks={}; region cache totals: hits={} refreshes={} bypasses={}; sample epochs belong to regions; output validated for requested TT={}; fixed TTL={} simulated seconds",
        selection.rows().len(), storage.motion.value().0.len(), storage.motion.value().1,
        storage.region_stats.hits, storage.region_stats.refreshes, storage.region_stats.bypasses, selection.epoch,
        storage.config.age_seconds(crate::cache::Group::StellarState)));
}
