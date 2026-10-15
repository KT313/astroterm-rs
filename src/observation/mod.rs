//! Step 2 of the frame: `observe`.
//!
//! Inputs: the observer's site, whether there is an atmosphere, the view and the simulated time. Output: the
//! observer's barycentric state, the rotation between catalog axes (J2000 equatorial) and the observer's horizon
//! (East, North, Up) in both directions, the refraction switch, and the patch of
//! catalog sky the view can show, margins included. Runs every frame; one Earth evaluation and a few rotations.
//! `apparent_direction` is the one place that turns a position or a star direction into what the observer sees.
use crate::time::StepTimes;
use crate::astro::models::orientation::{compute_body_fixed_rotation, compute_horizon_rotation, compute_site_state, compute_slow_orientation};
use crate::astro::models::{BodyId, BodyState, planets::evaluate_planet};
use crate::astro::{AstronomicalTime, LIGHT_SPEED_AU_DAY, Matrix3, Observer, Vector3, apply_aberration, refract_direction, remove_aberration, unrefract_direction};
use crate::constants::{ABERRATION_MARGIN, QUANTIZATION_MARGIN, REFRACTION_MARGIN, STELLAR_DRIFT_MARGIN};
use crate::model::View;
use std::f64::consts::PI;

const NUMERIC_SLACK: f64 = 1e-10;                                          // the old selection's padding, kept so the patch is never smaller than before

/// The observer and the sky at one simulated time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation {
    pub time_ms: i64,
    pub observer: BodyState,                // barycentric position and velocity of the site
    pub to_horizon: Matrix3,                // catalog axes → East, North, Up at the site
    pub to_catalog: Matrix3,                // horizon axes → catalog axes, used to select the visible sky patch
    pub atmosphere: bool,                   // apply refraction
    pub patch: SkyPatch,
}

/// The cone of catalog directions whose stars can appear in the view: the view's half field plus the margins for
/// star drift, aberration, refraction and region quantization. A radius of π means the whole sky.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyPatch {
    pub center: Vector3,                    // unit, catalog axes
    pub radius: f64,
}

/// Place the observer and orient the sky for `time`.
pub fn observe(site: &Observer, atmosphere: bool, view: &View, time_ms: i64, times: &mut StepTimes) -> Observation {
    let astronomical_time = AstronomicalTime::from_time_ms(time_ms);
    calculate_observation_at_dates(site, atmosphere, view, time_ms, &astronomical_time, times)
}

/// Numerical observer boundary, allowing reference fixtures to supply their original fractional UT1 and TT.
/// Only the integer timestamp is retained in the returned observation.
pub fn calculate_observation_at_dates(site: &Observer, atmosphere: bool, view: &View, time_ms: i64, astronomical_time: &AstronomicalTime, times: &mut StepTimes) -> Observation {
    // the observer in catalog axes: the Earth plus the site, rotated by the Earth's orientation
    times.measure("Observation.Earth state");
    let earth = evaluate_planet(BodyId::Earth, astronomical_time.tt_jd);
    times.measure("Observation.Observer and orientation");
    let (observer, to_horizon, to_catalog) = {
        let to_fixed = compute_body_fixed_rotation(compute_slow_orientation(astronomical_time.tt_jd), astronomical_time.ut1_jd);
        let from_fixed = to_fixed.transpose();
        let site_state = compute_site_state(*site);
        let offset = BodyState { position: from_fixed.apply(site_state.position), velocity: from_fixed.apply(site_state.velocity) };
        let observer = offset.add_parent(earth);
        let to_horizon = compute_horizon_rotation(site).compose(to_fixed);
        let to_catalog = to_horizon.transpose();
        (observer, to_horizon, to_catalog)
    };

    // the patch of catalog sky the view can show
    times.measure("Observation.Visible sky patch");
    let patch = {
        let margin = 0.1_f64.to_radians() / 3600.0 + STELLAR_DRIFT_MARGIN + QUANTIZATION_MARGIN + NUMERIC_SLACK
            + ABERRATION_MARGIN.max((observer.velocity.length() / LIGHT_SPEED_AU_DAY).clamp(0.0, 1.0).asin())
            + if atmosphere { REFRACTION_MARGIN } else { 0.0 };
        SkyPatch {
            center: to_catalog.apply(view.center_direction().to_unit_vector()).normalized(),
            radius: (view.fov_degrees.to_radians() / 2.0 + margin).min(PI),
        }
    };
    Observation { time_ms, observer, to_horizon, to_catalog, atmosphere, patch }
}

impl Observation {
    /// How a point at barycentric `position` (where its light left it) appears: a unit East, North, Up direction
    /// with aberration and, with an atmosphere, refraction.
    pub fn apparent_direction(&self, position: Vector3) -> Vector3 {
        self.apparent_star_direction((position - self.observer.position).normalized())
    }

    /// How a star in the unit catalog `direction` appears: aberration, rotation to the horizon, then refraction.
    pub fn apparent_star_direction(&self, direction: Vector3) -> Vector3 {
        let horizon = self.to_horizon.apply(apply_aberration(direction, self.observer.velocity));
        if self.atmosphere { refract_direction(horizon) } else { horizon }
    }

    /// The unit catalog direction a star must have to appear in the apparent `direction`: the inverse of
    /// `apparent_star_direction`, used by the warp's tests as the reference and by the warp per region.
    pub fn catalog_star_direction(&self, direction: Vector3) -> Vector3 {
        let horizon = if self.atmosphere { unrefract_direction(direction) } else { direction };
        remove_aberration(self.to_catalog.apply(horizon), self.observer.velocity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::J2000;
    use crate::model::{ProjectionKind, ViewCenter};

    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    fn view(fov_degrees: f64) -> View { View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.3 }, projection: ProjectionKind::Stereographic, fov_degrees } }

    #[test]
    fn the_observer_sits_on_the_earth_and_looks_up() {
        let observation = observe(&boston(), false, &View::default(), crate::astro::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
        let earth = evaluate_planet(BodyId::Earth, crate::astro::AstronomicalTime::from_time_ms(observation.time_ms).tt_jd);
        let offset = observation.observer.position - earth.position;
        assert!((offset.length() * 149_597_870.7 - 6368.0).abs() < 10.0, "{} km", offset.length() * 149_597_870.7); // Boston's distance from the geocentre
        let up = observation.to_horizon.apply(offset).normalized();
        assert!(up.z > 0.99999 && up.y < 0.0, "{up:?}");                                    // the geodetic vertical leans towards the equator by 0.19° at most
        let spin = (observation.observer.velocity - earth.velocity).length() * 149_597_870.7 / 86_400.0;
        assert!((spin - 0.343).abs() < 0.01, "{spin} km/s");                                 // 0.465 km/s × cos 42.36°
        assert!(((observation.observer.velocity.length() / LIGHT_SPEED_AU_DAY) - 1e-4).abs() < 1e-5);
        for axis in [Vector3 { x: 1.0, y: 0.0, z: 0.0 }, Vector3 { x: 0.0, y: 1.0, z: 0.0 }, Vector3 { x: 0.0, y: 0.0, z: 1.0 }] {
            let back = observation.to_catalog.apply(observation.to_horizon.apply(axis));
            assert!((back - axis).length() < 1e-14);
        }
    }

    #[test]
    fn a_target_straight_above_the_site_appears_at_the_zenith() {
        let observation = observe(&boston(), false, &View::default(), crate::astro::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
        let target = observation.observer.position + observation.to_catalog.apply(Vector3 { x: 0.0, y: 0.0, z: 1.0 }) * 1.0e6;
        let apparent = observation.apparent_direction(target);
        assert!((apparent.z - 1.0).abs() < 1e-7 && apparent.x.abs() < 2e-4, "{apparent:?}"); // aberration shifts by up to 20″
        let airless = Observation { atmosphere: false, ..observation };
        let direction = (target - observation.observer.position).normalized();
        assert_eq!(airless.apparent_direction(target), airless.apparent_star_direction(direction));
    }

    #[test]
    fn refraction_is_applied_once_after_the_rotation() {
        let (airless, refracted) = (observe(&boston(), false, &view(90.0), crate::astro::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default()), observe(&boston(), true, &view(90.0), crate::astro::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default()));
        let direction = Vector3 { x: 0.3, y: -0.5, z: 0.8 }.normalized();
        let low = airless.to_catalog.apply(Vector3 { x: 0.6, y: 0.0, z: 0.05 }.normalized());  // a star 3° above the horizon
        for star in [direction, low] {
            assert_eq!(refracted.apparent_star_direction(star), refract_direction(airless.apparent_star_direction(star)));
        }
        assert!(refracted.apparent_star_direction(low).z > airless.apparent_star_direction(low).z + 0.002); // lifted by about 14′
        assert_eq!(airless.observer, refracted.observer);
    }

    #[test]
    fn the_catalog_direction_inverts_the_apparent_one_with_and_without_atmosphere() {
        let time = crate::astro::utc_jd_to_time_ms(J2000).unwrap();
        for atmosphere in [false, true] {
            let observation = observe(&boston(), atmosphere, &view(90.0), time, &mut crate::time::StepTimes::default());
            for star in [Vector3 { x: 0.3, y: -0.5, z: 0.8 }, Vector3 { x: 0.6, y: 0.0, z: 0.05 }, Vector3 { x: -0.9, y: 0.1, z: -0.3 }] {
                let star = observation.to_catalog.apply(star.normalized());                  // horizon directions, high, low and below the horizon
                let back = observation.catalog_star_direction(observation.apparent_star_direction(star));
                assert!((back - star).length() < 0.01_f64.to_radians() / 3600.0, "atmosphere={atmosphere} {star:?}: {back:?}");
            }
        }
    }

    #[test]
    fn the_patch_covers_the_view_plus_the_margins() {
        let time = crate::astro::utc_jd_to_time_ms(J2000).unwrap();
        let airless = observe(&boston(), false, &view(90.0), time, &mut crate::time::StepTimes::default());
        let refracted = observe(&boston(), true, &view(90.0), time, &mut crate::time::StepTimes::default());
        let center = airless.to_horizon.apply(airless.patch.center);
        assert!((center - view(90.0).center_direction().to_unit_vector()).length() < 1e-14);
        let expected = 45_f64.to_radians() + STELLAR_DRIFT_MARGIN + ABERRATION_MARGIN + QUANTIZATION_MARGIN + 0.1_f64.to_radians() / 3600.0 + NUMERIC_SLACK;
        assert!((airless.patch.radius - expected).abs() < 1e-12);
        assert!((refracted.patch.radius - expected - REFRACTION_MARGIN).abs() < 1e-12);
        assert_eq!(observe(&boston(), true, &view(360.0), time, &mut crate::time::StepTimes::default()).patch.radius, PI);
        let zenith = observe(&boston(), false, &View::default(), time, &mut crate::time::StepTimes::default());
        assert!((zenith.to_horizon.apply(zenith.patch.center).z - 1.0).abs() < 1e-14);
    }
}
