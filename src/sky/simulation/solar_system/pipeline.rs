//! Refresh only the model families needed for the current time and requested light-emission times.
use crate::astro::models::BodyId;
use crate::model::{FrameTime, SimulationError, StateRequest};
use crate::state::SimulationState;
use crate::timing::StepTimes;
use super::samples::{refresh_planet_samples, refresh_lunar_samples, refresh_orientation_samples};

/// Ensure reception and explicitly requested emission epochs are covered, refreshing only missing families.
/// Observation preparation supplies observer-dependent light-time requests; all sample mutation stays here.
pub fn update_solar_system(state: &mut SimulationState, time: FrameTime, requests: &[StateRequest], times: &mut StepTimes) -> Result<(), SimulationError> {
    validate_sample_times(time, requests)?;                                 // reject non-finite simulation or requested times
    let reception = [time.tt];                                            // reception-only frames need no allocated time lists
    let requested_epochs;
    let (planet_epochs, moon_epochs) = if requests.is_empty() {
        (reception.as_slice(), reception.as_slice())
    } else {
        requested_epochs = collect_sample_times(time, requests);
        (requested_epochs.0.as_slice(), requested_epochs.1.as_slice())
    };
    let before = state.refresh_counts;

    refresh_planet_samples(state, planet_epochs, before, times)?;            // prepare Sun and planet positions wherever coverage is missing
    refresh_lunar_samples(state, moon_epochs, before, times)?;               // prepare the Moon's position relative to Earth
    refresh_orientation_samples(state, time.tt, before, times)?;             // prepare Earth's slowly changing axis direction
    Ok(())
}

fn validate_sample_times(time: FrameTime, requests: &[StateRequest]) -> Result<(), SimulationError> {
    if ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) || requests.iter().any(|r| !r.tt.is_finite()) {
        return Err(SimulationError::InvalidTime);
    }
    Ok(())
}

fn collect_sample_times(time: FrameTime, requests: &[StateRequest]) -> (Vec<f64>, Vec<f64>) {
    let mut planet_epochs = Vec::with_capacity(requests.len() + 1);         // every requested body needs a planetary sample at its emission time
    let mut moon_epochs = Vec::with_capacity(requests.iter().filter(|request| request.body == BodyId::Moon).count() + 1);
    planet_epochs.push(time.tt);
    moon_epochs.push(time.tt);
    for request in requests {
        planet_epochs.push(request.tt); // the Moon also needs its parent at emission, never at reception
        if request.body == BodyId::Moon {
            moon_epochs.push(request.tt);
        }
    }
    (planet_epochs, moon_epochs)
}
