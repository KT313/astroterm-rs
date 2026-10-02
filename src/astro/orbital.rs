//! Shared Kepler/orbital geometry. Angles in degrees in the inherited solvers; f64 throughout.
use crate::astro::Vector3;
use std::f64::consts::PI;
pub(crate) const TO_RAD: f64 = PI / 180.0;
pub(crate) const OBLIQUITY_J2000: f64 = 84381.448 / 3600.0 * TO_RAD;
/// Keplerian orbital elements, or their rates of change. Angles in degrees, semi-major axis in AU (Earth radii for
/// the Moon).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitalElements {
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub mean_anomaly: f64,
    pub argument_of_periapsis: f64,
    pub ascending_node: f64,
}

/// Elements at `time` units after their epoch, given their rates per unit.
pub(crate) fn propagate_elements(elements: &OrbitalElements, rates: &OrbitalElements, time: f64) -> OrbitalElements {
    OrbitalElements {
        semi_major_axis: elements.semi_major_axis + rates.semi_major_axis * time,
        eccentricity: elements.eccentricity + rates.eccentricity * time,
        inclination: elements.inclination + rates.inclination * time,
        mean_anomaly: elements.mean_anomaly + rates.mean_anomaly * time,
        argument_of_periapsis: elements.argument_of_periapsis + rates.argument_of_periapsis * time,
        ascending_node: elements.ascending_node + rates.ascending_node * time,
    }
}

/// Wrap an angle in degrees into [-180, 180).
pub(crate) fn wrap_degrees_signed(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// Solve Kepler's equation `M = E - e·sin(E)` (degrees) with Newton's method, at most 10 iterations.
pub(crate) fn solve_eccentric_anomaly(mean_anomaly: f64, eccentricity: f64, initial_guess: f64) -> f64 {
    let mut eccentric_anomaly = initial_guess;
    for _ in 0..10 {
        let mean_anomaly_error =
            mean_anomaly - (eccentric_anomaly - eccentricity / TO_RAD * (eccentric_anomaly * TO_RAD).sin());
        let correction = mean_anomaly_error / (1.0 - eccentricity * (eccentric_anomaly * TO_RAD).cos());
        eccentric_anomaly += correction;
        if correction.abs() <= 1e-6 {
            break;
        }
    }
    eccentric_anomaly
}

/// Position in the orbital plane, with the x-axis pointing at the periapsis.
pub(crate) fn compute_orbital_plane_position(
    semi_major_axis: f64,
    eccentricity: f64,
    eccentric_anomaly: f64,
) -> (f64, f64) {
    let e = eccentric_anomaly * TO_RAD;
    let xp = semi_major_axis * (e.cos() - eccentricity);
    let yp = semi_major_axis * (1.0 - eccentricity * eccentricity).sqrt() * e.sin();
    (xp, yp)
}

/// Rotate an orbital plane position to ecliptic coordinates.
pub(crate) fn rotate_orbital_plane_to_ecliptic(xp: f64, yp: f64, elements: &OrbitalElements) -> Vector3 {
    let (w, node, inclination) = (
        elements.argument_of_periapsis * TO_RAD,
        elements.ascending_node * TO_RAD,
        elements.inclination * TO_RAD,
    );
    let (sin_w, cos_w, sin_node, cos_node) = (w.sin(), w.cos(), node.sin(), node.cos());
    let (sin_i, cos_i) = (inclination.sin(), inclination.cos());
    Vector3 {
        x: (cos_w * cos_node - sin_w * sin_node * cos_i) * xp + (-sin_w * cos_node - cos_w * sin_node * cos_i) * yp,
        y: (cos_w * sin_node + sin_w * cos_node * cos_i) * xp + (-sin_w * sin_node + cos_w * cos_node * cos_i) * yp,
        z: (sin_w * sin_i) * xp + (cos_w * sin_i) * yp,
    }
}

/// Rotate ecliptic coordinates to equatorial coordinates at J2000.
pub(crate) fn ecliptic_to_equatorial(ecliptic: Vector3) -> Vector3 {
    let (sin_eps, cos_eps) = (OBLIQUITY_J2000.sin(), OBLIQUITY_J2000.cos());
    Vector3 {
        x: ecliptic.x,
        y: cos_eps * ecliptic.y - sin_eps * ecliptic.z,
        z: sin_eps * ecliptic.y + cos_eps * ecliptic.z,
    }
}

pub(crate) const fn elements(a: f64, e: f64, i: f64, m: f64, w: f64, node: f64) -> OrbitalElements {
    OrbitalElements {
        semi_major_axis: a,
        eccentricity: e,
        inclination: i,
        mean_anomaly: m,
        argument_of_periapsis: w,
        ascending_node: node,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrap_degrees_signed_handles_large_negative_angles() {
        assert!((wrap_degrees_signed(-1000.0) - 80.0).abs() < 1e-9);
        assert!((wrap_degrees_signed(190.0) + 170.0).abs() < 1e-9);
        assert!((wrap_degrees_signed(45.0) - 45.0).abs() < 1e-9);
    }

    #[test]
    fn solve_eccentric_anomaly_satisfies_keplers_equation() {
        let (mean_anomaly, eccentricity) = (40.0, 0.2);
        let solution = solve_eccentric_anomaly(mean_anomaly, eccentricity, mean_anomaly);
        let residual = solution - eccentricity / TO_RAD * (solution * TO_RAD).sin() - mean_anomaly;
        assert!(residual.abs() < 1e-6);
    }
}
