//! Observer geometry and explicit solar-system emission requests.
use crate::state::SimulationState;
use crate::model::{ObserverState, FrameTime, SimulationError, Anchor, PlanetKind, BodySamples};
use crate::astro::{Matrix3, Observer, LIGHT_SPEED_AU_DAY};
use crate::astro::models::{BodyId, BodyState, orientation::{compute_body_fixed_rotation, compute_horizon_rotation, compute_site_state}};
use crate::timing::StepTimes;
/// Combine the anchor and site states, then derive the observer's fixed and horizon rotations.
pub fn compose_observer_state(
    time: FrameTime,
    site: Observer,
    anchor_state: BodyState,
    inertial_to_fixed: Matrix3,
    site_fixed: BodyState,
    atmosphere: bool,
) -> ObserverState {
    let fixed_to_inertial = inertial_to_fixed.transpose();
    let offset = BodyState {
        position: fixed_to_inertial.apply(site_fixed.position),
        velocity: fixed_to_inertial.apply(site_fixed.velocity),
    };
    ObserverState {
        anchor: Anchor::Earth,
        site,
        height_m: 0.0,
        time,
        state: offset.add_parent(anchor_state),
        inertial_to_fixed,
        inertial_to_horizon: compute_horizon_rotation(&site).compose(inertial_to_fixed),
        atmosphere,
        emission_tt: [time.tt; 10],
    }
}

pub fn prepare_observer(
    simulation: &SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    let earth = crate::sky::evaluate_body(simulation, BodyId::Earth, time.tt)?;
    let slow = crate::sky::evaluate_orientation(simulation, time.tt)?;
    let orientation = compute_body_fixed_rotation(slow, time.ut1);
    Ok(crate::sky::compose_observer_state(
        time,
        site,
        earth,
        orientation,
        compute_site_state(site),
        true,
    ))
}


/// Plan observer-dependent emission epochs, then ask the simulation coordinator for coverage.
/// The target moves to emission time; the observer stays at reception. Two distance evaluations implement
/// the initial light-time estimate plus one iteration. No model is evaluated by observe_sky itself.
pub fn prepare_light_time_samples(
    simulation: &mut SimulationState,
    observer: &mut ObserverState,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let mut requests = Vec::with_capacity(9);
    for _ in 0..2 {
        requests.clear(); // reuse the buffer for the refined emission times
        for body in BodyId::PLANETS
            .into_iter()
            .chain([BodyId::Moon])
            .filter(|b| *b != BodyId::Earth)
        {
            let target = crate::sky::evaluate_body(simulation, body, observer.emission_tt[body as usize])?;
            let tt = observer.time.tt - (target.position - observer.state.position).length() / LIGHT_SPEED_AU_DAY;
            observer.emission_tt[body as usize] = tt;
            requests.push(crate::model::StateRequest { body, tt });
        }
        crate::sky::update_solar_system(simulation, observer.time, &requests, times)?;
    }
    Ok(())
}

/// Convenience coordinator for headless callers. Reception samples must already exist.
/// Production main.rs shows observer preparation and emission sampling explicitly.
pub fn prepare_observation(
    simulation: &mut SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    let mut observer = prepare_observer(simulation, time, site)?;
    prepare_light_time_samples(simulation, &mut observer, &mut StepTimes::default())?;
    Ok(observer)
}


pub(crate) fn sample_body_states(
    simulation: &SimulationState,
    observer: &ObserverState,
) -> Result<BodySamples, SimulationError> {
    let planets = PlanetKind::ALL
        .map(|kind| crate::sky::evaluate_body(simulation, body_id(kind), observer.emission_tt[body_id(kind) as usize]))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let moon = crate::sky::evaluate_body(simulation, crate::astro::models::BodyId::Moon,
        observer.emission_tt[crate::astro::models::BodyId::Moon as usize])?;
    Ok(BodySamples { planets, moon })
}
pub(super) fn body_id(kind: PlanetKind) -> BodyId {
    match kind {
        PlanetKind::Sun => BodyId::Sun,
        PlanetKind::Mercury => BodyId::Mercury,
        PlanetKind::Venus => BodyId::Venus,
        PlanetKind::Mars => BodyId::Mars,
        PlanetKind::Jupiter => BodyId::Jupiter,
        PlanetKind::Saturn => BodyId::Saturn,
        PlanetKind::Uranus => BodyId::Uranus,
        PlanetKind::Neptune => BodyId::Neptune,
    }
}
