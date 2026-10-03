//! Observer-dependent transformations and corrections. The preparation coordinator requests emission coverage;
//! observation itself reads immutable samples and never evaluates a model or sees a camera.
use super::{
    FrameTime, ObservedSky, ObservedStar, PlanetKind, SimulationError, SimulationState, refract_sky_positions,
};
use crate::astro::models::{
    BodyId, BodyState,
    orientation::{compute_body_fixed_rotation, compute_horizon_rotation, compute_site_state},
};
use crate::astro::{Matrix3, Observer, Vector3};
use crate::timing::StepTimes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Earth,
}

/// A frame's observer in the common inertial frame. Site coordinates are body-fixed; full orientation (slow and
/// fast) transforms the WGS84 sea-level site vector and its rotation velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObserverState {
    pub anchor: Anchor,
    pub site: Observer,
    pub height_m: f64,
    pub time: FrameTime,
    pub state: BodyState,
    pub inertial_to_fixed: Matrix3,
    pub inertial_to_horizon: Matrix3,
    pub atmosphere: bool,
    pub emission_tt: [f64; 10],
}
impl ObserverState {
    /// Compose a body's translation and complete orientation with a body-fixed site state. This geometry also
    /// serves synthetic anchors in tests; it assumes neither an Earth orbit nor spin around inertial Z.
    pub fn from_anchor_state(
        time: FrameTime,
        site: Observer,
        anchor_state: BodyState,
        inertial_to_fixed: Matrix3,
        site_fixed: BodyState,
        atmosphere: bool,
    ) -> Self {
        let fixed_to_inertial = inertial_to_fixed.transpose();
        let offset = BodyState {
            position: fixed_to_inertial.apply(site_fixed.position),
            velocity: fixed_to_inertial.apply(site_fixed.velocity),
        };
        Self {
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
}

pub fn prepare_observer(
    simulation: &SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    let earth = simulation.evaluate_body(BodyId::Earth, time.tt)?;
    let slow = simulation.evaluate_orientation(time.tt)?;
    let orientation = compute_body_fixed_rotation(slow, time.ut1);
    Ok(ObserverState::from_anchor_state(
        time,
        site,
        earth,
        orientation,
        compute_site_state(site),
        true,
    ))
}

/// Speed of light in AU/day (IAU exact metre definitions).
pub const LIGHT_SPEED_AU_DAY: f64 = 299792458.0 * 86400.0 / 149597870700.0;

/// Plan observer-dependent emission epochs, then ask the simulation coordinator for coverage.
/// The target moves to emission time; the observer stays at reception. Two distance evaluations implement
/// the initial light-time estimate plus one iteration. No model is evaluated by observe_sky itself.
pub fn prepare_light_time_samples(
    simulation: &mut SimulationState,
    observer: &mut ObserverState,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    for _ in 0..2 {
        let mut requests = Vec::with_capacity(9);
        for body in BodyId::PLANETS
            .into_iter()
            .chain([BodyId::Moon])
            .filter(|b| *b != BodyId::Earth)
        {
            let target = simulation.evaluate_body(body, observer.emission_tt[body as usize])?;
            let tt = observer.time.tt - (target.position - observer.state.position).length() / LIGHT_SPEED_AU_DAY;
            observer.emission_tt[body as usize] = tt;
            requests.push(super::StateRequest { body, tt });
        }
        super::update_simulation(simulation, observer.time, &requests, times)?;
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

/// First-order annual plus diurnal aberration; error is O(beta²), below 0.002 arcseconds for Earth.
pub fn apply_aberration(direction: Vector3, velocity: Vector3) -> Vector3 {
    (direction.normalized() + velocity * (1.0 / LIGHT_SPEED_AU_DAY)).normalized()
}

/// Evaluate region and brightness candidates and every body at the frame time. Corrections are applied once;
/// constellation endpoints are independent of region selection. Outside the interval selection is disabled.
pub fn observe_sky(
    simulation: &SimulationState,
    observer: &ObserverState,
    magnitude_threshold: f64,
    refraction: bool,
    region: super::SkyRegion,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let mut candidates = std::mem::take(&mut output.candidate_indices);
    output.selection = times.measure("Star selection", || {
        output.catalog.grid.select(
            &output.catalog.stars,
            region,
            observer,
            magnitude_threshold,
            refraction && observer.atmosphere,
            &mut candidates,
        )
    });
    let result = observe_sky_candidates(
        simulation,
        observer,
        magnitude_threshold,
        refraction,
        Some(&candidates),
        output,
        times,
    );
    output.candidate_indices = candidates;
    result
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
    // resolve all body dependencies before writing output, so missing coverage is explicit
    let tt = observer.time.tt;
    let bodies = PlanetKind::ALL
        .map(|kind| simulation.evaluate_body(body_id(kind), observer.emission_tt[body_id(kind) as usize]))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let moon = simulation.evaluate_body(BodyId::Moon, observer.emission_tt[BodyId::Moon as usize])?;
    let relative_moon = moon.position - observer.state.position;
    let relative_sun = bodies[0].position - observer.state.position;

    // union drawable candidates and constellation endpoints before applying any observer corrections
    times.measure("Stellar observation", || {
        let inside = crate::astro::COMPUTATIONAL_INTERVAL.contains(tt);
        let selected: Vec<_> = if !inside {
            (0..output.catalog.stars.len()).collect()
        } else if let Some(indices) = candidates {
            indices
                .iter()
                .copied()
                .filter(|&i| {
                    i < output.catalog.stars.len() && output.catalog.stars.brightness_key(i) <= magnitude_threshold
                })
                .collect()
        } else {
            (0..output.catalog.stars.len())
                .filter(|&i| output.catalog.stars.brightness_key(i) <= magnitude_threshold)
                .collect()
        };
        let mut selected = selected;
        selected.sort_unstable();
        selected.dedup();
        let mut candidates = selected.into_iter().peekable();
        let mut endpoints = output.catalog.endpoint_indices.iter().copied().peekable();
        output.magnitude_threshold = magnitude_threshold;
        output.stars.clear();
        output.runtime_singular_count = 0;
        let years = crate::astro::models::stars::years_since_j2000(tt);
        while candidates.peek().is_some() || endpoints.peek().is_some() {
            let index = candidates
                .peek()
                .copied()
                .unwrap_or(usize::MAX)
                .min(endpoints.peek().copied().unwrap_or(usize::MAX));
            let drawable = candidates.peek() == Some(&index);
            if drawable {
                candidates.next();
            }
            if endpoints.peek() == Some(&index) {
                endpoints.next();
            }
            let star = output.catalog.stars.get(index);
            let sample = star.motion.evaluate(years, star.magnitude);
            let mut observed = ObservedStar::from_star(
                &star,
                index,
                observer
                    .inertial_to_horizon
                    .apply(apply_aberration(sample.direction, observer.state.velocity)),
            );
            observed.magnitude = sample.magnitude;
            observed.drawable = drawable;
            observed.drawable &= sample.magnitude <= magnitude_threshold;
            output.runtime_singular_count += usize::from(sample.used_singular_fallback);
            output.stars.push(observed);
        }
    });

    // finite bodies share exact topocentric subtraction, aberration and orientation
    times.measure("Body observation", || {
        for (planet, state) in output.planets.iter_mut().zip(bodies) {
            planet.position = observer
                .inertial_to_horizon
                .apply(apply_aberration(
                    state.position - observer.state.position,
                    observer.state.velocity,
                ))
                .normalized();
        }
        output.moon.position = observer
            .inertial_to_horizon
            .apply(apply_aberration(relative_moon, observer.state.velocity));

        output.moon.illumination = super::compute_moon_illumination(
            relative_moon,
            relative_sun,
            crate::astro::models::orientation::j2000_ecliptic_north(),
        );
        output.moon.phase = output.moon.illumination.named_phase();
    });
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(tt);
    output.refracted = false;
    if refraction && observer.atmosphere {
        times.measure("Refraction", || refract_sky_positions(output));
    }
    Ok(())
}

fn body_id(kind: PlanetKind) -> BodyId {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aberration_points_towards_velocity_and_never_exceeds_its_geometric_bound() {
        let velocity = Vector3 {
            x: 0.0,
            y: 0.0176,
            z: 0.00027,
        };
        let beta = velocity.length() / LIGHT_SPEED_AU_DAY;
        for i in 0..1000 {
            let a = i as f64 * std::f64::consts::TAU / 1000.0;
            let direction = Vector3 {
                x: a.cos(),
                y: a.sin(),
                z: 0.0,
            };
            let apparent = apply_aberration(direction, velocity);
            let angle = direction.cross(apparent).length().atan2(direction.dot(apparent));
            assert!(angle <= beta.asin() + 1e-15);
            assert!(apparent.dot(velocity) >= direction.dot(velocity) - 1e-15);
        }
    }
    #[test]
    #[ignore = "release measurement of the phase-6 per-star correction"]
    fn measure_aberration_cost() {
        use std::hint::black_box;
        let directions: Vec<_> = (0..1_000_000)
            .map(|i| {
                let a = i as f64 * 0.001;
                Vector3 {
                    x: a.cos(),
                    y: a.sin(),
                    z: 0.2,
                }
                .normalized()
            })
            .collect();
        let start = std::time::Instant::now();
        for direction in &directions {
            black_box(apply_aberration(
                black_box(*direction),
                black_box(Vector3 {
                    x: 0.0,
                    y: 0.0176,
                    z: 0.00027,
                }),
            ));
        }
        println!(
            "aberration including input/output normalization: {:.3} ns/star (1,000,000 directions)",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
