//! Earth orientation, f64: Vondrak precession and IAU 2000B nutation use TT, fast ERA spin uses UT1.
//! The slow matrix includes the model-consistent equation of origins. WGS84 geometry supplies the site state.
mod long_term_terms;
mod nutation;
mod nutation_terms;
mod precession;
pub use nutation::compute_nutation;
mod site;
pub use precession::{PrecessionMatrix, compute_precession_matrix};
pub use site::compute_site_state;

use crate::astro::{J2000, Matrix3, Observer, earth_rotation_angle};

/// Mean obliquity implied by the long-term equator and ecliptic poles, radians.
pub fn compute_obliquity(tt: f64) -> f64 {
    precession::compute_equator_pole(tt)
        .dot(precession::compute_ecliptic_pole(tt))
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
    let integrand = |t: f64| {
        let epoch = J2000 + t * 36525.0;
        let p = precession::compute_equator_pole(epoch);
        let v = (precession::compute_equator_pole(epoch + 3.6525) - precession::compute_equator_pole(epoch - 3.6525))
            * 5000.0;
        (p.x * v.y - p.y * v.x) / (1.0 + p.z)
    };
    let mut integral = integrand(0.0) + integrand(centuries);
    for i in 1..panels {
        integral += (if i % 2 == 0 { 2.0 } else { 4.0 }) * integrand(i as f64 * h);
    }
    let angle = integral * h / 3.0 + (0.014506_f64 / 3600.0).to_radians();
    let p = precession::compute_equator_pole(tt);
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
