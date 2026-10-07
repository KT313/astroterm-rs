//! Cached observation order. Individual stages own their cache decisions, numerical work and diagnostics.
use crate::model::{ObservedSky, ObserverState, SimulationError};
use crate::state::{ObservationCache, SimulationState};
use crate::timing::StepTimes;
use super::caching::{
    reset_catalog_if_changed, capture_observation_reports, describe_observation_results,
    update_region_filtering, update_brightness_bounds, update_body_sampling, update_candidate_validation,
    update_constellation_endpoints, update_stellar_motion, update_current_brightness, update_correction_selection,
    update_observer_subtraction, update_moon_illumination, update_aberration, update_horizon_rotation, update_refraction,
};

/// Resolve all fallible body dependencies first; correction passes then publish a complete sky.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_sky(
    storage: &mut ObservationCache,
    simulation: &SimulationState,
    observer: &ObserverState,
    threshold: f64,
    refraction: bool,
    region: crate::model::SkyRegion,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    reset_catalog_if_changed(storage, &output.catalog); // discard results that belong to a different catalog
    let previous_reports = capture_observation_reports(storage, times);
    let epoch = observer.time.tt;

    // candidate membership is independent from intrinsic stellar cache lifetimes
    update_region_filtering(&mut storage.region, &storage.config, epoch, refraction, region, observer, &output.catalog.grid, times); // retain sky regions that may enter the view
    update_brightness_bounds(&mut storage.candidates, &storage.region, &storage.config, epoch, threshold, &output.catalog, times); // keep stars that could be bright enough to draw
    update_body_sampling(&mut storage.bodies, &storage.config, epoch, observer, simulation, times)?; // read Sun, planet and Moon positions at their light-emission times

    output.selection = storage.candidates.value().1; // publish conservative selection counts
    update_candidate_validation(&mut storage.selected, &storage.candidates, &storage.config, epoch, threshold, &output.catalog, times); // validate selected indices and brightness bounds
    update_constellation_endpoints(&mut storage.working, &storage.selected, &storage.config, epoch, output.catalog.endpoint_indices(), times); // include stars needed by constellation lines
    update_stellar_motion(storage, epoch, output, times); // update selected stars using their catalog motion
    update_current_brightness(&mut storage.eligible, &storage.working, &storage.motion, &storage.config, epoch, threshold, &mut output.magnitude_threshold, times); // mark stars bright enough at the current simulated time

    update_correction_selection(&storage.working, &storage.eligible, &mut storage.corrections, &storage.motion, &storage.config, epoch, output, times); // retain drawable stars and constellation endpoints

    // each cache owns a distinct coordinate-space result
    update_observer_subtraction(&mut storage.relative, &storage.bodies, &storage.config, epoch, observer, output, times); // calculate body positions relative to the viewer
    update_moon_illumination(&mut storage.illumination, &storage.config, epoch, output, times); // calculate the illuminated part of the Moon
    update_aberration(&storage.motion, &storage.relative, &storage.corrections, &mut storage.apparent, &storage.config, epoch, observer, output, times); // correct apparent directions for the viewer’s velocity
    update_horizon_rotation(&storage.apparent, &mut storage.horizontal, &storage.config, epoch, observer, output, times); // turn apparent directions into local sky directions
    update_refraction(&storage.horizontal, &mut storage.refracted, &storage.config, epoch, refraction && observer.atmosphere, output, times); // apply atmospheric bending when enabled
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(epoch); // record whether this date exceeds the supported interval
    describe_observation_results(storage, output, threshold, previous_reports, times); // report counts without repeating calculations
    Ok(())
}
