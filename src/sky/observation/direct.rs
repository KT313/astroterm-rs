//! Direct observation and observer geometry, without reusable correction caches.
use super::stages::*;
use crate::state::SimulationState;

use crate::model::{ObservedSky, PlanetKind, FrameTime, SimulationError};
use crate::sky::refract_sky_positions;
use crate::astro::models::{
    BodyId, BodyState,
    orientation::{compute_body_fixed_rotation, compute_horizon_rotation, compute_site_state},
};
use crate::astro::{Matrix3, Observer, Vector3};
use crate::timing::StepTimes;

use crate::model::{Anchor, ObserverState};

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
            let target = crate::sky::evaluate_body(simulation, body, observer.emission_tt[body as usize])?;
            let tt = observer.time.tt - (target.position - observer.state.position).length() / LIGHT_SPEED_AU_DAY;
            observer.emission_tt[body as usize] = tt;
            requests.push(crate::model::StateRequest { body, tt });
        }
        crate::sky::update_simulation(simulation, observer.time, &requests, times)?;
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

/// Aberration for an already normalized stellar direction. `beta` is observer velocity / c, prepared once per pass.
/// The near-unit output has a bounded squared norm, so a square root suffices; retain robust normalization for
/// unusual synthetic velocities/cancellation. The generic distance-vector API above retains both normalizations.
pub(super) fn apply_unit_aberration(direction: Vector3, beta: Vector3) -> Vector3 {
    debug_assert!(
        (direction.dot(direction) - 1.0).abs() < 1e-12,
        "stellar direction must be unit length"
    );
    let apparent = direction + beta;
    let squared_norm = apparent.dot(apparent);
    if (0.5..=2.0).contains(&squared_norm) {
        apparent * (1.0 / squared_norm.sqrt())
    } else {
        apparent.normalized()
    }
}

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
    // resolve every body dependency before changing output; all following passes are infallible
    let tt = observer.time.tt;
    let bodies = times.measure("Body sampling", || sample_body_states(simulation, observer))?;

    // preserve conservative eligibility separately from current brightness and constellation membership
    let selected = times.measure("Candidate validation", || {
        filter_brightness_candidates(&output.catalog, tt, magnitude_threshold, candidates)
    });
    times.measure("Constellation endpoints", || {
        include_constellation_endpoints(selected, output)
    });
    times.measure("Stellar motion", || evaluate_stellar_motion(tt, output));
    times.measure("Current brightness", || {
        filter_current_magnitudes(magnitude_threshold, output)
    });

    times.measure("Correction selection", || {
        select_corrections(output);
    });

    // form observer-relative geometry before the direction-only corrections
    let (relative_moon, relative_sun) = times.measure("Observer subtraction", || {
        subtract_observer_position(bodies, observer, output)
    });
    times.measure("Moon illumination", || {
        update_moon_illumination(relative_moon, relative_sun, output)
    });
    times.measure("Aberration", || apply_sky_aberration(observer.state.velocity, output));
    times.measure("Horizon rotation", || {
        rotate_sky_to_horizon(observer.inertial_to_horizon, output)
    });

    // publish coverage and optionally refract the completed horizontal directions
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(tt);
    output.refracted = false;
    if refraction && observer.atmosphere {
        times.measure("Refraction", || refract_sky_positions(output));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_aberration_matches_generic_correction_and_preserves_normalization() {
        let mut maximum = 0.0_f64;
        for i in 0..10000 {
            let a = i as f64 * 0.013;
            let b = i as f64 * 0.029;
            let direction = Vector3 {
                x: a.cos() * b.cos(),
                y: a.sin() * b.cos(),
                z: b.sin(),
            }
            .normalized();
            for velocity in [
                Vector3::default(),
                Vector3 {
                    x: 0.0176,
                    y: -0.001,
                    z: 0.0003,
                },
                Vector3 {
                    x: -0.0004,
                    y: 0.017,
                    z: -0.001,
                },
                Vector3 {
                    x: 1000.0,
                    y: -500.0,
                    z: 0.0,
                },
                Vector3 {
                    x: 1e200,
                    y: 1e200,
                    z: 0.0,
                },
            ] {
                let expected = apply_aberration(direction, velocity);
                let actual = apply_unit_aberration(direction, velocity * (1.0 / LIGHT_SPEED_AU_DAY));
                let error = expected.cross(actual).length().atan2(expected.dot(actual)).to_degrees() * 3600.0;
                maximum = maximum.max(error);
                assert!(error < 1e-8, "aberration delta {error} arcseconds");
                assert!((actual.dot(actual) - 1.0).abs() < 2e-15);
            }
        }
        println!("maximum unit-aberration delta: {maximum:.12e} arcseconds");
    }

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
        let velocity = Vector3 {
            x: 0.0,
            y: 0.0176,
            z: 0.00027,
        };
        let beta = velocity * (1.0 / LIGHT_SPEED_AU_DAY);
        for unit_input in [false, true] {
            let start = std::time::Instant::now();
            for direction in &directions {
                black_box(if unit_input {
                    apply_unit_aberration(black_box(*direction), black_box(beta))
                } else {
                    apply_aberration(black_box(*direction), black_box(velocity))
                });
            }
            println!(
                "unit_input={unit_input}: {:.3} ns/star (1,000,000 directions)",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
