//! Count existing intermediate buffers only in the opt-in trace; never rerun astronomy or selection.
use super::*;

impl ObservationCache {
    pub(super) fn describe_observation(&self, output: &ObservedSky, threshold: f64, times: &mut StepTimes) {
        if times.trace().is_none() {
            return;
        }
        let total = output.catalog.stars.len();
        let (cells, regional, always) = output.catalog.grid.count_region_stars(self.region.value(), total);
        times.describe("Region filtering", || format!("input stars={total}; selected cells={cells}/{}; retained by conservative region including always-checked={regional}; rejected region={}; always-checked subset={always}; brute-force={}", crate::sky::grid::CELL_COUNT, total - regional, output.selection.brute_force));
        times.describe("Brightness bounds", || format!("input regional stars={regional}; rejected interval magnitude bound > {threshold}={}; output candidates={}; brute-force bypass={}", regional - self.candidates.value().0.len(), self.candidates.value().0.len(), output.selection.brute_force));
        times.describe("Body sampling", || {
            format!(
                "requested Sun/planets={}; Moon=1; output states={} at emission epochs",
                output.planets.len(),
                output.planets.len() + 1
            )
        });
        times.describe("Candidate validation", || {
            let candidates = &self.candidates.value().0;
            let invalid = candidates.iter().filter(|&&i| i >= total).count();
            let removed = candidates.len() - self.selected.value().len();
            format!("input candidates={}; rejected invalid index={invalid}; then rejected bound > {threshold}={}; output candidates={}; outside interval uses all stars", candidates.len(), removed - invalid, self.selected.value().len())
        });
        let working = self.working.value();
        times.describe("Constellation endpoints", || format!("input selected={}; endpoint union={}; added endpoint-only={}; output working stars={}; endpoints included even when constellation drawing is disabled", self.selected.value().len(), output.catalog.endpoint_indices.len(), working.len() - self.selected.value().len(), working.len()));
        times.describe("Stellar motion", || format!("input working stars={}; output directions/magnitudes={}; singular fallbacks={}; stellar sample cache totals: hits={} refreshes={} bypasses={}; stars use catalog propagation, no per-star light-time solve", working.len(), self.motion.value().0.len(), output.runtime_singular_count, self.stellar_stats.hits, self.stellar_stats.refreshes, self.stellar_stats.bypasses));
        let drawable = self.eligible.value().iter().filter(|&&yes| yes).count();
        times.describe("Current brightness", || {
            let eligible = working.iter().filter(|s| s.drawable).count();
            format!("input working stars={}; excluded from drawing by candidate membership={}; then rejected current magnitude > {threshold}={}; drawable={drawable}; output flags={} (no records removed yet)", working.len(), working.len()-eligible, eligible-drawable, self.eligible.value().len())
        });
        times.describe("Correction selection", || format!("input working stars={}; retained drawable={drawable}; additionally retained endpoint-only={}; removed faint non-endpoints={}; output corrected stars={}", working.len(), output.corrections.endpoint_only, output.corrections.skipped, output.stars.len()));
        times.describe("Observer subtraction", || {
            format!(
                "input/output body vectors={}; barycentric positions -> observer-relative AU; stars unchanged={}",
                output.planets.len() + 1,
                output.stars.len()
            )
        });
        times.describe("Moon illumination", || {
            format!(
                "input Moon + Sun relative vectors=2; output Moon=1; illuminated fraction={:.8}; phase={:?}",
                output.moon.illumination.illuminated_fraction, output.moon.phase
            )
        });
        for (name, description) in [
            ("Aberration", "inertial directions -> apparent directions"),
            ("Horizon rotation", "apparent directions -> East/North/Up"),
            (
                "Refraction",
                "airless horizontal directions -> refracted horizontal directions",
            ),
        ] {
            times.describe(name, || {
                format!(
                    "input/output stars={}; Sun/planets={}; Moon=1; {description}; no membership filtering",
                    output.stars.len(),
                    output.planets.len()
                )
            });
        }
    }
}
