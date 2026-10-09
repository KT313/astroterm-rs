//! Count existing intermediate buffers only in the opt-in trace; never rerun astronomy or selection.
use super::*;

pub(super) fn describe_observation(stars: crate::state::StellarResults<'_>, output: &ObservedSky, threshold: f64, times: &mut StepTimes) {
    if times.trace().is_none() {
        return;
    }
    let working = stars.selection.working.value();
    let drawable = output.corrections.evaluated - output.corrections.skipped - output.corrections.endpoint_only; // retained stars are drawable stars plus faint constellation endpoints
    times.describe("Current brightness", || {
        let eligible = stars.selection.statistics.candidates;
        format!("input working stars={}; excluded from drawing by candidate membership={}; then rejected current magnitude > {threshold}={}; drawable={drawable}; regional flags={} (no combined flag list)", working.len(), working.len()-eligible, eligible-drawable, output.corrections.evaluated)
    });
    times.describe("Correction selection", || format!("input working stars={}; retained drawable={drawable}; additionally retained endpoint-only={}; removed faint non-endpoints={}; output corrected stars={}", working.len(), output.corrections.endpoint_only, output.corrections.skipped, output.corrections.evaluated - output.corrections.skipped));
    times.describe("Observer subtraction", || {
        format!(
            "input/output body vectors={}; barycentric positions -> observer-relative AU; stars unchanged={}",
            output.planets.len() + 1,
            output.corrections.evaluated - output.corrections.skipped
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
                output.corrections.evaluated - output.corrections.skipped,
                output.planets.len()
            )
        });
    }
}

pub(in crate::sky::observation) struct ObservationReports {
    caches: Vec<crate::cache::CacheReport>,
    regional: [crate::cache::CacheStats; 3], // brightness, selection and aberration have independent per-region caches
}

pub(in crate::sky::observation) fn capture_observation_reports(storage: &ObservationCache, times: &mut StepTimes) -> Option<ObservationReports> {
    let mut reports = None;
    times.measure_diagnostics(|_| reports = Some(ObservationReports { caches: storage.reports(), regional: storage.region_stats }));
    reports
}

pub(in crate::sky::observation) fn describe_observation_results(storage: &ObservationCache, stars: crate::state::StellarResults<'_>, output: &ObservedSky, threshold: f64, previous_reports: Option<ObservationReports>, times: &mut StepTimes) {
    times.measure_diagnostics(|times| {
        diagnostics::describe_observation(stars, output, threshold, times);
        if let Some(previous) = previous_reports {
            for (index, name) in ["Current brightness", "Correction selection", "Aberration"].into_iter().enumerate() {
                let before = previous.regional[index];
                let after = storage.region_stats[index];
                times.describe(name, || format!("regional cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; no combined result cache",
                    after.hits - before.hits, after.refreshes - before.refreshes, after.bypasses - before.bypasses, after.last_reason));
            }
            for (before, after) in previous.caches.into_iter().zip(storage.reports()) {
                times.describe(after.name, || format!("cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; stored TT={:?}; validity={} s", after.stats.hits - before.stats.hits, after.stats.refreshes - before.stats.refreshes, after.stats.bypasses - before.stats.bypasses, after.stats.last_reason, after.calculated_at, after.valid_seconds));
            }
        }
    });
}


