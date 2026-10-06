//! Independently refreshed planetary, lunar and orientation samples. No observer or camera lives here.
//! Linear intervals control interpolation error only, not the underlying ephemerides' astronomical accuracy.
mod pipeline;
mod samples;
mod memory;
pub use pipeline::update_simulation;

use crate::state::SimulationState;
use crate::astro::models::{BodyId, BodyState};
use crate::astro::Matrix3;
use crate::model::{ModelFamily, SimulationError, Sample};

/// Read-only same-time evaluation; missing coverage is a coordinator error, never a hidden ephemeris call.
pub fn evaluate_body(storage: &SimulationState, body: BodyId, tt: f64) -> Result<BodyState, SimulationError> {
    if body == BodyId::Moon {
        let sample = find_sample(&storage.moon, tt, ModelFamily::Moon)?;
        let relative = sample.value.evaluate(tt - sample.epoch);
        return Ok(relative.add_parent(evaluate_body(storage, BodyId::Earth, tt)?));
    }
    let sample = find_sample(&storage.planets, tt, ModelFamily::Planets)?;
    Ok(sample.value[body as usize].evaluate(tt - sample.epoch))
}

pub fn evaluate_orientation(storage: &SimulationState, tt: f64) -> Result<Matrix3, SimulationError> {
    Ok(find_sample(&storage.orientation, tt, ModelFamily::Orientation)?.value)
}

fn find_sample<T>(samples: &[Sample<T>], tt: f64, family: ModelFamily) -> Result<&Sample<T>, SimulationError> {
    samples
        .iter()
        .find(|sample| sample.covers(tt))
        .ok_or(SimulationError::MissingCoverage { family, tt })
}

