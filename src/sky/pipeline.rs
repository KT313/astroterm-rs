//! Headless compatibility coordinators: select, simulate and observe through separate domain functions.
use crate::state::SimulationState;
use crate::model::{ObservedSky, ObserverState, SimulationError};
use crate::timing::StepTimes;
use crate::sky::{sample_body_states, filter_brightness_candidates, merge_constellation_endpoints, simulate_stars_direct, apply_direct_observation};

/// Evaluate region and brightness candidates and every body at the frame time. Corrections are applied once;
/// constellation endpoints are independent of region selection. Outside the interval selection is disabled.
pub fn observe_sky(
    simulation: &SimulationState,
    observer: &ObserverState,
    magnitude_threshold: f64,
    refraction: bool,
    region: crate::model::SkyRegion,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let bodies = times.measure("Body sampling", || sample_body_states(simulation, observer))?;
    let mut candidates = std::mem::take(&mut output.candidate_indices);
    let selected_region = times.measure("Region filtering", || {
        crate::sky::select_region(&output
            .catalog
            .grid, region, observer, refraction && observer.atmosphere)
    });
    output.selection = times.measure("Brightness bounds", || {
        crate::sky::select_brightness(&output.catalog.grid, &output.catalog.stars,
            &selected_region,
            magnitude_threshold,
            &mut candidates)
    });
    observe_prepared_candidates(bodies, observer, magnitude_threshold, refraction, Some(&candidates), output, times);
    output.candidate_indices = candidates;
    Ok(())
}

/// Optional catalog indices limit candidate work, not constellation endpoints or the values computed for them.
/// Outside the computational interval all stars are checked, since interval-specific selection is invalid there.
pub fn observe_sky_candidates(
    simulation: &SimulationState,
    observer: &ObserverState,
    magnitude_threshold: f64,
    refraction: bool,
    candidates: Option<&[usize]>,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let bodies = times.measure("Body sampling", || sample_body_states(simulation, observer))?;
    observe_prepared_candidates(bodies, observer, magnitude_threshold, refraction, candidates, output, times);
    Ok(())
}

fn observe_prepared_candidates(bodies: crate::model::BodySamples, observer: &ObserverState, magnitude_threshold: f64,
    refraction: bool, candidates: Option<&[usize]>, output: &mut ObservedSky, times: &mut StepTimes) {
    let tt = observer.time.tt;
    let selected = times.measure("Candidate validation", || filter_brightness_candidates(&output.catalog, tt, magnitude_threshold, candidates));
    let working = times.measure_steps("Constellation endpoints", |times| merge_constellation_endpoints(selected, output.catalog.endpoint_indices(), times));
    let (motion, singular) = times.measure("Stellar motion", || simulate_stars_direct(output.catalog.stars.borrow_stellar_fields(), &working, tt));
    apply_direct_observation(&working, &motion, singular, bodies, observer, magnitude_threshold, refraction, output, times);
}
