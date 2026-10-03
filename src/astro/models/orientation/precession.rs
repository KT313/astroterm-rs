//! Precession: the slow turning of the Earth's axis, which moves the celestial equator and equinox against the stars.
//!
//! Vondrak, Capitaine & Wallace (2011/2012), translated from ERFA ltp/ltpequ/ltpecl.
//! Copyright (C) 2013-2023 NumFOCUS Foundation; see LICENSE-ERFA.

use std::f64::consts::PI;

use crate::astro::{Equatorial, J2000, Vector3, rectangular_to_equatorial};

/// Rotation from the mean equator and equinox of J2000 to those of a date.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrecessionMatrix([[f64; 3]; 3]);

/// The precession rotation from J2000 to `julian_date_tt`.
pub fn compute_precession_matrix(julian_date_tt: f64) -> PrecessionMatrix {
    let equator = compute_equator_pole(julian_date_tt);
    let ecliptic = compute_ecliptic_pole(julian_date_tt);
    let equinox = equator.cross(ecliptic).normalized();
    let y = equator.cross(equinox);
    PrecessionMatrix([
        [equinox.x, equinox.y, equinox.z],
        [y.x, y.y, y.z],
        [equator.x, equator.y, equator.z],
    ])
}

/// Evaluate the polynomial plus periodic pole coordinates in radians, ERFA ltpequ/ltpecl.
fn compute_pole_components(tt: f64, polynomial: &[[f64; 4]; 2], periodic: &[[f64; 5]]) -> (f64, f64) {
    let t = (tt - J2000) / 36525.0;
    let mut xy = [0.0; 2];
    for row in periodic {
        let (s, c) = (std::f64::consts::TAU * t / row[0]).sin_cos();
        for i in 0..2 {
            xy[i] += c * row[1 + i] + s * row[3 + i];
        }
    }
    for i in 0..2 {
        xy[i] += polynomial[i].iter().rev().fold(0.0, |v, c| v * t + c);
        xy[i] *= PI / (180.0 * 3600.0);
    }
    (xy[0], xy[1])
}

pub(super) fn compute_equator_pole(tt: f64) -> Vector3 {
    use super::long_term_terms::{XYPER, XYPOL};
    let (x, y) = compute_pole_components(tt, &XYPOL, &XYPER);
    Vector3 {
        x,
        y,
        z: (1.0 - x * x - y * y).max(0.0).sqrt(),
    }
}

pub(super) fn compute_ecliptic_pole(tt: f64) -> Vector3 {
    use super::long_term_terms::{PQPER, PQPOL};
    let (p, q) = compute_pole_components(tt, &PQPOL, &PQPER);
    let w = (1.0 - p * p - q * q).max(0.0).sqrt();
    let (s, c) = (84381.406 * PI / (180.0 * 3600.0)).sin_cos();
    Vector3 {
        x: p,
        y: -q * c - w * s,
        z: -q * s + w * c,
    }
}

impl PrecessionMatrix {
    pub fn matrix(self) -> crate::astro::Matrix3 {
        crate::astro::Matrix3(self.0)
    }

    /// Rotate rectangular J2000 equatorial coordinates to the equator and equinox of date.
    pub fn apply(&self, position: Vector3) -> Vector3 {
        let [x, y, z] = self
            .0
            .map(|row| row[0] * position.x + row[1] * position.y + row[2] * position.z);
        Vector3 { x, y, z }
    }

    /// Precess a J2000 right ascension and declination to the equator and equinox of date.
    pub fn apply_to_equatorial(&self, position: Equatorial) -> Equatorial {
        rectangular_to_equatorial(self.apply(position.to_unit_vector()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TO_RAD: f64 = PI / 180.0;

    #[test]
    fn precession_at_j2000_is_the_identity() {
        let PrecessionMatrix(matrix) = compute_precession_matrix(J2000);
        for (i, row) in matrix.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1e-9, "[{i}][{j}] = {value}");
            }
        }
    }

    #[test]
    fn precession_matrix_is_a_rotation() {
        for julian_date in [J2000 - 365250.0, J2000 + 9000.0, J2000 + 365250.0] {
            let PrecessionMatrix(m) = compute_precession_matrix(julian_date);
            for i in 0..3 {
                for j in 0..3 {
                    let dot: f64 = (0..3).map(|k| m[k][i] * m[k][j]).sum();
                    let expected = if i == j { 1.0 } else { 0.0 };
                    assert!((dot - expected).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn precession_matches_meeus_example() {
        // Meeus example 21.b: θ Persei (proper motion already applied) precessed to 2028-11-13.19 TD
        let precession = compute_precession_matrix(2462088.69);
        let theta_persei = Equatorial {
            right_ascension: 41.054063 * TO_RAD,
            declination: 49.227750 * TO_RAD,
        };
        let of_date = precession.apply_to_equatorial(theta_persei);
        let arcsecond = TO_RAD / 3600.0;
        assert!(
            (of_date.right_ascension - 41.547214 * TO_RAD).abs() < arcsecond,
            "{of_date:?}"
        );
        assert!(
            (of_date.declination - 49.348483 * TO_RAD).abs() < arcsecond,
            "{of_date:?}"
        );
    }
}
