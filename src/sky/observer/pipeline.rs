//! Resolve reception geometry, emission epochs and final body samples before star work.
use crate::{astro::Observer, model::{FrameTime, ObserverState, SimulationError}, state::{SimulationState, ObserverPreparationCache}, timing::StepTimes};
use super::{prepare_cached_observer_with_times, prepare_cached_light_time, prepare_cached_bodies};

pub fn prepare_observer_inputs(storage: &mut ObserverPreparationCache, simulation: &mut SimulationState,
    time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<ObserverState, SimulationError> {
    if let Some(observer) = super::group::reuse_completed_observer(simulation, storage, time, site) { return Ok(observer); }
    super::group::ensure_solar_request(simulation, storage, time, site, times)?; // reception and emission share one request
    let result = finish_observer_preparation(storage, simulation, time, site, times);
    if result.is_err() { simulation.abort_request(); }
    result
}

fn finish_observer_preparation(storage: &mut ObserverPreparationCache, simulation: &mut SimulationState,
    time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<ObserverState, SimulationError> {
    let mut observer = prepare_cached_observer_with_times(storage, simulation, time, site, times)?; // locate the viewer at reception time
    super::diagnostics::describe_geometry(time, site, times);
    times.measure_steps("Light-time sampling", |times| prepare_cached_light_time(storage, simulation, &mut observer, times))?; // sample when each body's light left it
    super::diagnostics::describe_emissions(&observer, storage, times);
    prepare_cached_bodies(storage, simulation, &observer, times)?; // finish every fallible body lookup before star processing
    simulation.complete_request(simulation.request_key(time, site, storage.source_id())); // publish only after every fallible lookup succeeded
    storage.complete_solar_request(simulation.request_token());
    Ok(observer)
}
