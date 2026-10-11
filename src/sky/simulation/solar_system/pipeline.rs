//! Refresh only the model families needed for the current time and requested light-emission times.
use crate::astro::models::BodyId;
use crate::model::{FrameTime, SimulationError, StateRequest};
use crate::state::SimulationState;
use crate::timing::StepTimes;
use super::samples::{refresh_planet_samples, refresh_lunar_samples, refresh_orientation_samples};

/// Ensure reception and explicitly requested emission epochs are covered, refreshing only missing families.
/// Observation preparation supplies observer-dependent light-time requests; all sample mutation stays here.
pub fn update_solar_system(state: &mut SimulationState, time: FrameTime, requests: &[StateRequest], times: &mut StepTimes) -> Result<(), SimulationError> {
    let result = prepare_requested_samples(state, time, requests, times);
    if result.is_err() { state.abort_request(); }
    else if result == Ok(true) { state.invalidate(); } // changed sample coverage/order requires preparing the complete body result again
    result.map(|_| ())
}

fn prepare_requested_samples(state: &mut SimulationState, time: FrameTime, requests: &[StateRequest], times: &mut StepTimes) -> Result<bool, SimulationError> {
    validate_sample_times(time, requests)?;                                 // reject non-finite simulation or requested times
    let reception = [time.tt];                                            // reception-only frames need no allocated time lists
    let requested_epochs;
    let moon_epochs = if requests.iter().all(|request| request.body != BodyId::Moon) {
        reception.as_slice()
    } else {
        requested_epochs = collect_lunar_epochs(time, requests);
        requested_epochs.as_slice()
    };
    let before = state.refresh_counts;

    let planets = refresh_planet_samples(state, time.tt, requests, before, times)?; // prepare each body wherever its own coverage is missing
    let moon = refresh_lunar_samples(state, moon_epochs, before, times)?;       // prepare the Moon's position relative to Earth
    let orientation = refresh_orientation_samples(state, time.tt, before, times)?; // prepare Earth's slowly changing axis direction
    Ok(planets || moon || orientation)
}

fn validate_sample_times(time: FrameTime, requests: &[StateRequest]) -> Result<(), SimulationError> {
    if ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) || requests.iter().any(|r| !r.tt.is_finite()) {
        return Err(SimulationError::InvalidTime);
    }
    Ok(())
}

/// Reception first, then the Moon's own emission epochs; planetary epochs are collected per body during their refresh.
fn collect_lunar_epochs(time: FrameTime, requests: &[StateRequest]) -> Vec<f64> {
    let mut moon_epochs = Vec::with_capacity(requests.iter().filter(|request| request.body == BodyId::Moon).count() + 1);
    moon_epochs.push(time.tt);
    moon_epochs.extend(requests.iter().filter(|request| request.body == BodyId::Moon).map(|request| request.tt));
    moon_epochs
}
