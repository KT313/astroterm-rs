//! Cached observation order. Individual stages own their cache decisions, numerical work and diagnostics.
use crate::model::{ObservedSky, ObserverState};
use crate::state::{ApparentDirections, ObservationCache};
use crate::timing::StepTimes;
use super::caching::{
    capture_observation_reports, describe_observation_results,
    update_regional_brightness, update_regional_corrections,
    update_observer_subtraction, update_moon_illumination, update_regional_aberration, update_horizon_rotation, update_refraction,
};

/// Apply corrections to complete, matching inputs. This stage never requests simulation or selects catalog rows.
#[allow(clippy::too_many_arguments)]
fn prepare_cached_observation(
    storage: &mut ObservationCache, stars: crate::state::StellarResults<'_>, bodies: crate::state::PreparedBodies<'_>,
    observer: &ObserverState, threshold: f64, refraction: bool, output: &mut ObservedSky, times: &mut StepTimes,
) {
    super::caching::prepare_inputs(storage, stars, bodies, observer, &output.catalog); // validate matching inputs and reset stale corrections
    storage.published = None;
    let previous_reports = capture_observation_reports(storage, times);
    let epoch = observer.time.tt;

    output.selection = stars.selection.statistics;
    output.runtime_singular_count = stars.fallback_count();
    update_regional_brightness(storage, stars, threshold, output, times); // mark stars bright enough at the current simulated time

    update_regional_corrections(storage, stars, output, times); // retain drawable stars and constellation endpoints

    // each cache owns a distinct coordinate-space result
    update_observer_subtraction(&mut storage.relative, bodies.cache, &storage.config, epoch, observer, output, times); // calculate body positions relative to the viewer
    update_moon_illumination(&mut storage.illumination, &storage.config, epoch, output, times); // calculate the illuminated part of the Moon
    update_regional_aberration(storage, stars, observer, times); // correct apparent directions for the viewer’s velocity
    let apparent = ApparentDirections::new(&storage.regional_output, &storage.regions, &storage.body_apparent); // borrow saved star and body directions without copying
    update_horizon_rotation(apparent, &mut storage.horizontal_sources, &mut storage.horizontal, &mut storage.horizontal_work, &storage.config, epoch, observer.inertial_to_horizon, times); // turn apparent directions into local sky directions
    update_refraction(&storage.horizontal, &mut storage.refracted, &mut storage.refraction_work, &storage.config, epoch, refraction && observer.atmosphere, times); // apply atmospheric bending when enabled
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(epoch); // record whether this date exceeds the supported interval
    storage.use_refraction = refraction && observer.atmosphere;
    output.refracted = storage.use_refraction;
    storage.published = Some(stars.publication_key());
    describe_observation_results(storage, stars, output, threshold, previous_reports, times); // report counts without repeating calculations
}

/// Publish the completed frame with region provenance; callers cannot mutate the sky while this view exists.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_regions<'a>(storage: &'a mut ObservationCache, stars: crate::state::StellarResults<'a>, bodies: crate::state::PreparedBodies<'_>,
    observer: &ObserverState, threshold: f64, refraction: bool, output: &'a mut ObservedSky, times: &mut StepTimes,
) -> crate::state::RegionalObservation<'a> {
    output.stars = Vec::new(); // release any old explicitly materialized output; the production path owns no star rows here
    prepare_cached_observation(storage, stars, bodies, observer, threshold, refraction, output, times);
    crate::state::RegionalObservation { sky: storage.observed_view(stars, output), regions: &storage.regional_output, owner: storage.identity,
        horizon: observer.inertial_to_horizon, refraction: refraction && observer.atmosphere }
}

/// Explicit compatibility output for callers that require an owned sky. The production loop uses borrowed regions.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_sky(storage: &mut ObservationCache, stars: crate::state::StellarResults<'_>, bodies: crate::state::PreparedBodies<'_>,
    observer: &ObserverState, threshold: f64, refraction: bool, output: &mut ObservedSky, times: &mut StepTimes,
) {
    prepare_cached_observation(storage, stars, bodies, observer, threshold, refraction, output, times);
    let owned = times.measure("Observed output materialization", || storage.observed_view(stars, output).materialize());
    times.record_build(crate::timing::BufferId::ObservedStars, || crate::timing::BufferShape::vector(&owned.stars, crate::timing::IndexDomain::Observed));
    *output = owned;
}
