//! Cached observation order. Individual stages own their cache decisions, numerical work and diagnostics.
use crate::model::{ObservedSky, ObserverState};
use crate::state::{ObservationCache};
use crate::timing::StepTimes;
use super::caching::{
    capture_observation_reports, describe_observation_results,
    update_regional_brightness, update_regional_corrections,
    update_observer_subtraction, update_moon_illumination, update_regional_aberration, update_horizon_rotation, update_refraction,
};

/// Apply corrections to complete, matching inputs. This stage never requests simulation or selects catalog rows.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_sky(
    storage: &mut ObservationCache, stars: crate::state::StellarResults<'_>, bodies: crate::state::PreparedBodies<'_>,
    observer: &ObserverState, threshold: f64, refraction: bool, output: &mut ObservedSky, times: &mut StepTimes,
) {
    super::caching::prepare_inputs(storage, stars, bodies, observer, &output.catalog); // validate matching inputs and reset stale corrections
    let motion = stars.motion;
    let previous_reports = capture_observation_reports(storage, times);
    let epoch = observer.time.tt;

    output.selection = stars.selection.statistics;
    output.runtime_singular_count = motion.value().1;
    update_regional_brightness(storage, stars, threshold, output, times); // mark stars bright enough at the current simulated time

    update_regional_corrections(storage, stars, output, times); // retain drawable stars and constellation endpoints

    // each cache owns a distinct coordinate-space result
    update_observer_subtraction(&mut storage.relative, bodies.cache, &storage.config, epoch, observer, output, times); // calculate body positions relative to the viewer
    update_moon_illumination(&mut storage.illumination, &storage.config, epoch, output, times); // calculate the illuminated part of the Moon
    update_regional_aberration(storage, stars, observer, output, times); // correct apparent directions for the viewer’s velocity
    update_horizon_rotation(&storage.apparent, &mut storage.horizontal, &storage.config, epoch, observer, output, times); // turn apparent directions into local sky directions
    update_refraction(&storage.horizontal, &mut storage.refracted, &storage.config, epoch, refraction && observer.atmosphere, output, times); // apply atmospheric bending when enabled
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(epoch); // record whether this date exceeds the supported interval
    describe_observation_results(storage, stars, output, threshold, previous_reports, times); // report counts without repeating calculations
}

/// Publish the completed frame with region provenance; callers cannot mutate the sky while this view exists.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_regions<'a>(storage: &'a mut ObservationCache, stars: crate::state::StellarResults<'_>, bodies: crate::state::PreparedBodies<'_>,
    observer: &ObserverState, threshold: f64, refraction: bool, output: &'a mut ObservedSky, times: &mut StepTimes,
) -> crate::state::RegionalObservation<'a> {
    observe_cached_sky(storage, stars, bodies, observer, threshold, refraction, output, times);
    crate::state::RegionalObservation { sky: output, regions: &storage.regional_output, owner: storage.identity,
        horizon: observer.inertial_to_horizon, refraction: refraction && observer.atmosphere }
}
