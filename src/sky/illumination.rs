//! Continuous illumination from observer-relative vectors in one inertial frame. The phase angle is independent
//! of the anchor. Waxing means positive projected elongation around the supplied reference-plane north pole.
use crate::astro::{MoonPhase, Vector3, moon_age_to_phase};
use std::f64::consts::{PI, TAU};

use crate::model::MoonIllumination;

/// Resolve the named phase from continuous phase angle and waxing direction.
pub fn name_moon_phase(illumination: MoonIllumination) -> MoonPhase {
    if illumination.phase_angle <= 0.03 * TAU {
        return MoonPhase::Full;
    } // phase latitude means exact i=0 is rare
    let fraction = (PI - illumination.phase_angle) / TAU;
    moon_age_to_phase(if illumination.waxing { fraction } else { 1.0 - fraction })
}

/// `moon` and `sun` are relative to the same observer at the same epoch. The observer-to-Moon vector is negated
/// to form the Moon-to-observer direction; the Sun-to-Moon separation is never confused with elongation.
pub fn compute_moon_illumination(moon: Vector3, sun: Vector3, reference_north: Vector3) -> MoonIllumination {
    let to_observer = -moon;
    let to_sun = sun - moon;
    let cosine = (to_observer.dot(to_sun) / (to_observer.length() * to_sun.length())).clamp(-1.0, 1.0);
    MoonIllumination {
        illuminated_fraction: (1.0 + cosine) / 2.0,
        phase_angle: cosine.acos(),
        waxing: sun.cross(moon).dot(reference_north) >= 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_new_quarter_and_waxing_use_the_angle_at_the_moon() {
        let moon = Vector3 { x: 1.0, y: 0.0, z: 0.0 };
        let north = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let full = compute_moon_illumination(moon, -moon, north);
        let new = compute_moon_illumination(moon, moon * 2.0, north);
        let quarter = compute_moon_illumination(
            moon,
            Vector3 {
                x: 1.0,
                y: -1.0,
                z: 0.0,
            },
            north,
        );
        let waning = compute_moon_illumination(moon, Vector3 { x: 1.0, y: 1.0, z: 0.0 }, north);
        assert_eq!((full.illuminated_fraction, crate::sky::name_moon_phase(full)), (1.0, MoonPhase::Full));
        assert_eq!((new.illuminated_fraction, crate::sky::name_moon_phase(new)), (0.0, MoonPhase::New));
        assert_eq!(quarter.illuminated_fraction, 0.5);
        assert_eq!(crate::sky::name_moon_phase(quarter), MoonPhase::FirstQuarter);
        assert!(quarter.waxing && !waning.waxing);
        assert_eq!(crate::sky::name_moon_phase(waning), MoonPhase::LastQuarter);
    }
}
