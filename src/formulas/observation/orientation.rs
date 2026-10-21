//! Earth orientation, f64: Vondrak precession and IAU 2000B nutation use TT, fast ERA spin uses UT1.
//! The slow matrix includes the model-consistent equation of origins. WGS84 geometry supplies the site state.
use std::f64::consts::PI;
use crate::formulas::math::evaluate_polynomial;
use crate::formulas::{J2000, Matrix3, Observer, Vector3, Equatorial, PrecessionMatrix, earth_rotation_angle, rectangular_to_equatorial};
use crate::formulas::constants::{OBLIQUITY_J2000, XYPER, XYPOL, PQPER, PQPOL, NUTATION_TERMS};

// Long-term precession: Vondrak, Capitaine & Wallace (2011/2012), translated from ERFA ltp/ltpequ/ltpecl.
// Copyright (C) 2013-2023 NumFOCUS Foundation; see LICENSE-ERFA.
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
        xy[i] += evaluate_polynomial(t, &polynomial[i]);
        xy[i] *= PI / (180.0 * 3600.0);
    }
    (xy[0], xy[1])
}

fn compute_equator_pole(tt: f64) -> Vector3 {
    let (x, y) = compute_pole_components(tt, &XYPOL, &XYPER);
    Vector3 {
        x,
        y,
        z: (1.0 - x * x - y * y).max(0.0).sqrt(),
    }
}

fn compute_ecliptic_pole(tt: f64) -> Vector3 {
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
    pub fn matrix(self) -> crate::formulas::Matrix3 {
        crate::formulas::Matrix3(self.0)
    }

    /// Rotate rectangular J2000 equatorial coordinates to the equator and equinox of date.
    pub fn apply(&self, position: Vector3) -> Vector3 {
        self.matrix().apply(position)
    }

    /// Precess a J2000 right ascension and declination to the equator and equinox of date.
    pub fn apply_to_equatorial(&self, position: Equatorial) -> Equatorial {
        rectangular_to_equatorial(self.apply(position.to_unit_vector()))
    }
}

// Short-period nutation: IAU 2000B, translated from ERFA nut00b (77 terms).
// Copyright (C) 2013-2023 NumFOCUS Foundation; see LICENSE-ERFA.
/// Nutation in longitude and obliquity in radians at TT. Fixed planetary offsets follow IAU 2000B.
/// Its short-model linear fundamental arguments are retained, including for unvalidated extrapolation.
pub fn compute_nutation(tt: f64) -> (f64, f64) {
    let t = (tt - J2000) / 36525.0;
    let as2r = std::f64::consts::PI / (180.0 * 3600.0);
    let argument_coefficients = [
        (485868.249036, 1717915923.2178),
        (1287104.79305, 129596581.0481),
        (335779.526232, 1739527262.8478),
        (1072260.70369, 1602961601.2090),
        (450160.398036, -6962890.5431),
    ];
    let mut arguments = [0.0; 5];
    for (argument, (a, b)) in arguments.iter_mut().zip(argument_coefficients) {
        *argument = (a + b * t).rem_euclid(1296000.0) * as2r;
    }
    let (mut psi, mut eps) = (0.0, 0.0);
    for row in NUTATION_TERMS.iter().rev() {
        let mut products = [0.0; 5];
        for (index, product) in products.iter_mut().enumerate() {
            *product = row[index] * arguments[index];
        }
        let argument: f64 = products.into_iter().sum();
        let (s, c) = argument.sin_cos();
        psi += (row[5] + row[6] * t) * s + row[7] * c;
        eps += (row[8] + row[9] * t) * c + row[10] * s;
    }
    ((psi / 1e7 - 0.000135) * as2r, (eps / 1e7 + 0.000388) * as2r)
}

// Combine precession, nutation and Earth rotation to orient the observer's sky.
/// Rotate ecliptic coordinates to equatorial coordinates at J2000.
pub(crate) fn ecliptic_to_equatorial(ecliptic: Vector3) -> Vector3 {
    let (sin_eps, cos_eps) = (OBLIQUITY_J2000.sin(), OBLIQUITY_J2000.cos());
    Vector3 {
        x: ecliptic.x,
        y: cos_eps * ecliptic.y - sin_eps * ecliptic.z,
        z: sin_eps * ecliptic.y + cos_eps * ecliptic.z,
    }
}

/// Mean obliquity implied by the long-term equator and ecliptic poles, radians.
pub fn compute_obliquity(tt: f64) -> f64 {
    compute_equator_pole(tt)
        .dot(compute_ecliptic_pole(tt))
        .clamp(-1.0, 1.0)
        .acos()
}

/// Passive X rotation, with the same convention as Matrix3::rotate_z.
pub fn rotate_x(angle: f64) -> Matrix3 {
    let (s, c) = angle.sin_cos();
    Matrix3([[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]])
}

/// Long-term mean equation of origins. Parallel-transport the origin along this model's equator pole,
/// then measure its angle from this model's equinox. This avoids extrapolating the IAU 2006 GMST polynomial.
/// Composite Simpson quadrature uses at most half-century panels; the pole derivative uses a symmetric 0.01 yr
/// step. J2000's ERA origin offset is the IAU 2006 value. Nutation is added separately, exactly once.
pub fn compute_mean_equation_of_origins(tt: f64) -> f64 {
    let centuries = (tt - J2000) / 36525.0;
    let panels = ((centuries.abs() * 2.0).ceil() as usize)
        .clamp(2, 20000)
        .next_multiple_of(2);
    let h = centuries / panels as f64;
    let mut integral = compute_origin_integrand(0.0) + compute_origin_integrand(centuries);
    for i in 1..panels {
        integral += (if i % 2 == 0 { 2.0 } else { 4.0 }) * compute_origin_integrand(i as f64 * h);
    }
    let angle = integral * h / 3.0 + (0.014506_f64 / 3600.0).to_radians();
    let p = compute_equator_pole(tt);
    let a = 1.0 / (1.0 + p.z);
    let basis = Matrix3([
        [1.0 - a * p.x * p.x, -a * p.x * p.y, -p.x],
        [-a * p.x * p.y, 1.0 - a * p.y * p.y, -p.y],
        [p.x, p.y, p.z],
    ]);
    let origin = Matrix3::rotate_z(angle).compose(basis);
    let relative = origin.compose(compute_precession_matrix(tt).matrix().transpose());
    -relative.0[0][1].atan2(relative.0[0][0])
}

/// Contribution of the moving equator pole to the equation-of-origins integral.
fn compute_origin_integrand(centuries: f64) -> f64 {
    // Evaluate the pole and its symmetric derivative at this integration sample.
    let epoch = J2000 + centuries * 36525.0;
    let pole = compute_equator_pole(epoch);
    let derivative = (compute_equator_pole(epoch + 3.6525) - compute_equator_pole(epoch - 3.6525)) * 5000.0;

    // Measure how the pole's motion turns the transported reference direction.
    (pole.x * derivative.y - pole.y * derivative.x) / (1.0 + pole.z)
}

/// Slow true-equator rotation C = R3(−EO) N P, sampled in TT. No Earth spin here.
pub fn compute_slow_orientation(tt: f64) -> Matrix3 {
    let (dpsi, deps) = compute_nutation(tt);
    let eps = compute_obliquity(tt);
    let nutation = rotate_x(-eps - deps)
        .compose(Matrix3::rotate_z(-dpsi))
        .compose(rotate_x(eps));
    let eo = compute_mean_equation_of_origins(tt) - dpsi * eps.cos();
    Matrix3::rotate_z(-eo)
        .compose(nutation)
        .compose(compute_precession_matrix(tt).matrix())
}

/// Body-fixed equatorial axes to East/North/Up at a site; independent of which body carries the site.
pub fn compute_horizon_rotation(site: &Observer) -> Matrix3 {
    let (sl, cl) = site.longitude.sin_cos();
    let (sp, cp) = site.latitude.sin_cos();
    Matrix3([[-sl, cl, 0.0], [-sp * cl, -sp * sl, cp], [cp * cl, cp * sl, sp]])
}

/// Fast UT1 spin applied to an independently sampled slow TT orientation.
pub fn compute_body_fixed_rotation(slow: Matrix3, julian_date_ut1: f64) -> Matrix3 {
    Matrix3::rotate_z(earth_rotation_angle(julian_date_ut1)).compose(slow)
}

/// Reference north for waxing/waning in common J2000 coordinates. Illumination itself is frame-independent.
pub fn j2000_ecliptic_north() -> crate::formulas::Vector3 {
    let (s, c) = OBLIQUITY_J2000.sin_cos();
    crate::formulas::Vector3 { x: 0.0, y: -s, z: c }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::{COMPUTATIONAL_INTERVAL, greenwich_mean_sidereal_time};
    use proptest::prelude::*;
    use crate::formulas::constants::DEGREES_TO_RADIANS as TO_RAD;

    proptest! {
        #[test]
        fn slow_fast_split_matches_sidereal_rotation(
            tt in COMPUTATIONAL_INTERVAL.start_tt..COMPUTATIONAL_INTERVAL.end_tt,
            delta in -1.0_f64..1.0, lat in -1.57_f64..1.57, lon in -std::f64::consts::PI..std::f64::consts::PI
        ) {
            let ut1 = tt + delta;
            let fast = Matrix3::rotate_z(earth_rotation_angle(ut1));
            let split = fast.compose(Matrix3::rotate_z(-compute_mean_equation_of_origins(tt)));
            let old = Matrix3::rotate_z(greenwich_mean_sidereal_time(ut1, tt));
            let h = compute_horizon_rotation(&Observer { latitude: lat, longitude: lon });
            let p = compute_precession_matrix(tt).matrix();
            for (a, b) in [(split, old), (h.compose(split).compose(p), h.compose(old).compose(p))] {
                for i in 0..3 { for j in 0..3 { prop_assert!((a.0[i][j] - b.0[i][j]).abs() < 1e-12); } }
            }
        }
    }


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
                    let products = [m[0][i] * m[0][j], m[1][i] * m[1][j], m[2][i] * m[2][j]];
                    let dot: f64 = products.into_iter().sum();
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
