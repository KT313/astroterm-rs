//! Precession: the slow turning of the Earth's axis, which moves the celestial equator and equinox against the stars.
//!
//! References: Capitaine, Wallace & Chapront, "Expressions for IAU 2000 precession quantities" (2003), eq. 39 (the
//! IAU 2006 angles); Jean Meeus, Astronomical Algorithms, ch. 21.

use std::f64::consts::PI;

use super::{Equatorial, J2000, Vector3, rectangular_to_equatorial};

/// Rotation from the mean equator and equinox of J2000 to those of a date.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrecessionMatrix([[f64; 3]; 3]);

/// The precession rotation from J2000 to `julian_date_tt`.
pub fn compute_precession_matrix(julian_date_tt: f64) -> PrecessionMatrix {
    // precession angles ζ, z and θ in arcseconds, from Julian centuries since J2000
    let t = (julian_date_tt - J2000) / 36525.0;
    let zeta = 2.650545 + 2306.083227 * t + 0.2988499 * t.powi(2) + 0.01801828 * t.powi(3)
        - 0.000005971 * t.powi(4)
        - 0.0000003173 * t.powi(5);
    let z = -2.650545 + 2306.077181 * t + 1.0927348 * t.powi(2) + 0.01826837 * t.powi(3)
        - 0.000028596 * t.powi(4)
        - 0.0000002904 * t.powi(5);
    let theta = 2004.191903 * t
        - 0.4294934 * t.powi(2)
        - 0.04182264 * t.powi(3)
        - 0.000007089 * t.powi(4)
        - 0.0000001274 * t.powi(5);

    // the rotation R3(-z) · R2(θ) · R3(-ζ), written out
    let to_radians = |arcseconds: f64| arcseconds / 3600.0 * PI / 180.0;
    let (sin_zeta, cos_zeta) = to_radians(zeta).sin_cos();
    let (sin_z, cos_z) = to_radians(z).sin_cos();
    let (sin_theta, cos_theta) = to_radians(theta).sin_cos();
    PrecessionMatrix([
        [
            cos_zeta * cos_theta * cos_z - sin_zeta * sin_z,
            -sin_zeta * cos_theta * cos_z - cos_zeta * sin_z,
            -sin_theta * cos_z,
        ],
        [
            cos_zeta * cos_theta * sin_z + sin_zeta * cos_z,
            -sin_zeta * cos_theta * sin_z + cos_zeta * cos_z,
            -sin_theta * sin_z,
        ],
        [cos_zeta * sin_theta, -sin_zeta * sin_theta, cos_theta],
    ])
}

impl PrecessionMatrix {
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
