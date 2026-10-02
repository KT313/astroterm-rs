//! Earth orientation, f64: precession uses TT, fast spin uses UT1. The slow matrix carries the mean equation of
//! origins, so its equinox agrees with the sidereal-time model. No nutation or finite observer site yet.
mod precession;
pub use precession::{PrecessionMatrix, compute_precession_matrix};

use crate::astro::{J2000, Matrix3, Observer, earth_rotation_angle};

/// Mean equation of the origins, ERA − GMST, radians (TT). Kept unwrapped to avoid subtracting large angles.
pub fn compute_mean_equation_of_origins(julian_date_tt: f64) -> f64 {
    let t = (julian_date_tt - J2000) / 36525.0;
    let precession_arcsec = -0.014506 - 4612.156534 * t - 1.3915817 * t.powi(2)
        + 0.00000044 * t.powi(3)
        + 0.000029956 * t.powi(4)
        + 0.0000000368 * t.powi(5);
    precession_arcsec / 3600.0 * std::f64::consts::PI / 180.0
}

/// Slow J2000-to-intermediate rotation C = R3(−EO_mean) P, sampled in TT.
pub fn compute_slow_orientation(julian_date_tt: f64) -> Matrix3 {
    Matrix3::rotate_z(-compute_mean_equation_of_origins(julian_date_tt))
        .compose(compute_precession_matrix(julian_date_tt).matrix())
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
pub fn j2000_ecliptic_north() -> crate::astro::Vector3 {
    let (s, c) = crate::astro::orbital::OBLIQUITY_J2000.sin_cos();
    crate::astro::Vector3 { x: 0.0, y: -s, z: c }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{COMPUTATIONAL_INTERVAL, greenwich_mean_sidereal_time};
    use proptest::prelude::*;

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
}
