//! One request spans reception samples, both light-time iterations and final body sampling.
use crate::{astro::Observer, constants::SOLAR_SYSTEM_GROUP_TTL_SECONDS, model::{FrameTime, ObserverState, SimulationError},
    state::{SimulationState, ObserverPreparationCache}, timing::StepTimes};

/// Begin the always-requested solar group before observer preparation; viewing controls are not inputs.
pub fn begin_solar_system_frame(simulation: &mut SimulationState, observer: &mut ObserverPreparationCache,
    time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<(), SimulationError> {
    if ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) { simulation.abort_request(); return Err(SimulationError::InvalidTime); }
    let reuse = times.measure("Solar-system cache decision", || reuse_completed_observer(simulation, observer, time, site).is_some());
    times.describe("Solar-system cache decision", || format!("complete group reused={reuse}; TTL={SOLAR_SYSTEM_GROUP_TTL_SECONDS} simulated seconds; observer location and model policies are dependencies"));
    if reuse { return Ok(()); }
    let key = simulation.request_key(time, site, observer.source_id());
    simulation.start_request(key);                             // clear sample lengths, retaining both allocations for each family
    observer.invalidate_solar_request();                       // emission epochs from a previous request must not survive a restart
    crate::sky::update_solar_system(simulation, time, &[], times)
}

pub(super) fn reuse_completed_observer(simulation: &SimulationState, observer: &ObserverPreparationCache,
    time: FrameTime, site: Observer) -> Option<ObserverState> {
    let key = simulation.request_key(time, site, observer.source_id());
    if !simulation.permits_reuse() || !observer.permits_solar_reuse() || simulation.group.has_been_invalidated { return None; }
    if simulation.group.complete_key != Some(key) { return None; }
    if (time.tt - simulation.group.calculated_at?).abs() * 86400.0 > SOLAR_SYSTEM_GROUP_TTL_SECONDS { return None; }
    observer.completed_observer(simulation.request_token()).filter(|result| result.time == time && result.site == site)
}

pub(super) fn ensure_solar_request(simulation: &mut SimulationState, observer: &mut ObserverPreparationCache,
    time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<(), SimulationError> {
    let key = simulation.request_key(time, site, observer.source_id());
    if simulation.group.request_key != Some(key) || observer.solar_request.is_some() {
        begin_solar_system_frame(simulation, observer, time, site, times)?; // also supports callers without an explicit reception-stage call
    }
    Ok(())
}
