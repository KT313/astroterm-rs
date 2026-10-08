//! Count existing intermediate buffers only in the opt-in trace; never rerun astronomy or selection.
use super::*;

pub(super) fn describe_observation(storage: &ObservationCache, stars: crate::state::StellarResults<'_>, output: &ObservedSky, threshold: f64, times: &mut StepTimes) {
    if times.trace().is_none() {
        return;
    }
    let working = stars.selection.working.value();
    let drawable = output.stars.len() - output.corrections.endpoint_only; // retained stars are drawable stars plus faint constellation endpoints
    times.describe("Current brightness", || {
        let eligible = working.iter().filter(|s| s.drawable).count();
        format!("input working stars={}; excluded from drawing by candidate membership={}; then rejected current magnitude > {threshold}={}; drawable={drawable}; output flags={} (no records removed yet)", working.len(), working.len()-eligible, eligible-drawable, storage.eligible.value().len())
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

pub(in crate::sky::observation) fn capture_observation_reports(storage: &ObservationCache, times: &mut StepTimes) -> Option<Vec<crate::cache::CacheReport>> {
    let mut reports = None;
    times.measure_diagnostics(|_| reports = Some(storage.reports()));
    reports
}

pub(in crate::sky::observation) fn describe_observation_results(storage: &ObservationCache, stars: crate::state::StellarResults<'_>, output: &ObservedSky, threshold: f64, previous_reports: Option<Vec<crate::cache::CacheReport>>, times: &mut StepTimes) {
    times.measure_diagnostics(|times| {
        diagnostics::describe_observation(storage, stars, output, threshold, times);
        if let Some(previous) = previous_reports {
            for (before, after) in previous.into_iter().zip(storage.reports()) {
                times.describe(after.name, || format!("cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; stored TT={:?}; validity={} s", after.stats.hits - before.stats.hits, after.stats.refreshes - before.stats.refreshes, after.stats.bypasses - before.stats.bypasses, after.stats.last_reason, after.calculated_at, after.valid_seconds));
            }
        }
    });
}


