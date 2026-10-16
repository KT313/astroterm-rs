//! Meeus chapter 47 (truncated ELP-2000/82), geometric mean ecliptic of date, TT, f64.
//! Conversion uses the date's obliquity and inverse long-term precession to form an Earth-relative J2000
//! equatorial AU state. The coordinator composes Earth's translation at the requested epoch.
use crate::formulas::{J2000, Vector3, MoonPhase, MoonIllumination};
use crate::formulas::constants::{MOON_PHASE_NAMES, MOON_PHASE_UPPER_BOUNDS, MOON_LONGITUDE_DISTANCE, MOON_LATITUDE};

/// Geometric mean ecliptic longitude, latitude and distance at TT, in radians and kilometres.
/// Truncated ELP-2000/82, Meeus second edition chapter 47; all arithmetic is f64.
pub fn compute_lunar_ecliptic(tt: f64) -> (f64, f64, f64) {
    // fundamental arguments, reduced before trigonometry
    let t = (tt - J2000) / 36525.0;
    let radians = |degrees: f64| degrees.rem_euclid(360.0).to_radians();
    let l =
        radians(218.3164477 + 481267.88123421 * t - 0.0015786 * t * t + t.powi(3) / 538841.0 - t.powi(4) / 65194000.0);
    let d =
        radians(297.8501921 + 445267.1114034 * t - 0.0018819 * t * t + t.powi(3) / 545868.0 - t.powi(4) / 113065000.0);
    let m = radians(357.5291092 + 35999.0502909 * t - 0.0001536 * t * t + t.powi(3) / 24490000.0);
    let mp =
        radians(134.9633964 + 477198.8675055 * t + 0.0087414 * t * t + t.powi(3) / 69699.0 - t.powi(4) / 14712000.0);
    let f =
        radians(93.2720950 + 483202.0175233 * t - 0.0036539 * t * t - t.powi(3) / 3526000.0 + t.powi(4) / 863310000.0);
    let e = 1.0 - 0.002516 * t - 0.0000074 * t * t;
    let argument = |row: &[f64]| row[0] * d + row[1] * m + row[2] * mp + row[3] * f;

    // periodic longitude, distance and latitude
    let (mut longitude, mut distance, mut latitude) = (0.0, 0.0, 0.0);
    for row in MOON_LONGITUDE_DISTANCE {
        let (s, c) = argument(&row).sin_cos();
        let factor = e.powi(row[1].abs() as i32);
        longitude += row[4] * factor * s;
        distance += row[5] * factor * c;
    }
    for row in MOON_LATITUDE {
        latitude += row[4] * e.powi(row[1].abs() as i32) * argument(&row).sin();
    }

    // additive planetary/figure-of-Earth terms, in the tables' microdegree units
    let a1 = radians(119.75 + 131.849 * t);
    let a2 = radians(53.09 + 479264.290 * t);
    let a3 = radians(313.45 + 481266.484 * t);
    longitude += 3958.0 * a1.sin() + 1962.0 * (l - f).sin() + 318.0 * a2.sin();
    latitude +=
        -2235.0 * l.sin() + 382.0 * a3.sin() + 175.0 * (a1 - f).sin() + 175.0 * (a1 + f).sin() + 127.0 * (l - mp).sin()
            - 115.0 * (l + mp).sin();
    (
        l + (longitude / 1e6).to_radians(),
        (latitude / 1e6).to_radians(),
        385000.56 + distance / 1000.0,
    )
}

impl MoonPhase {
    /// Human readable name, e.g. "Waxing Crescent".
    pub fn name(self) -> &'static str {
        MOON_PHASE_NAMES[self as usize]
    }
}

/// Named phase for a Moon age in [0, 1).
pub fn moon_age_to_phase(age: f64) -> MoonPhase {
    if !(0.03..=0.97).contains(&age) {
        return MoonPhase::New;
    }
    let index = MOON_PHASE_UPPER_BOUNDS.iter().position(|&bound| age < bound).unwrap_or(7);
    MoonPhase::ALL[index]
}

/// Resolve the named phase from the continuous phase angle and the waxing direction.
pub fn name_moon_phase(illumination: MoonIllumination) -> MoonPhase {
    use std::f64::consts::{PI, TAU};
    if illumination.phase_angle <= 0.03 * TAU {
        return MoonPhase::Full;
    } // phase latitude means exact i=0 is rare
    let fraction = (PI - illumination.phase_angle) / TAU;
    moon_age_to_phase(if illumination.waxing { fraction } else { 1.0 - fraction })
}

/// `moon` and `sun` are relative to the same observer at the same epoch. The observer-to-Moon vector is negated
/// to form the Moon-to-observer direction; the Sun-to-Moon separation is never confused with elongation. Waxing
/// means positive projected elongation around `reference_north` (the J2000 ecliptic pole for the Earth).
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

/// Parent-relative common-frame lunar state. Its only frame dependency is the source precession evaluated at
/// each finite-difference epoch; Earth's translation is composed at the requested epoch by the coordinator.
pub fn evaluate_moon(julian_date_tt: f64) -> crate::formulas::BodyState {
    crate::formulas::math::evaluate_with_velocity(julian_date_tt, |tt| {
        let (longitude, latitude, distance_km) = compute_lunar_ecliptic(tt);
        let (sl, cl) = longitude.sin_cos();
        let (sb, cb) = latitude.sin_cos();
        let native = Vector3 {
            x: cb * cl,
            y: cb * sl,
            z: sb,
        } * (distance_km / 149597870.7);
        let eps = crate::formulas::compute_obliquity(tt);
        crate::formulas::compute_precession_matrix(tt)
            .matrix()
            .transpose()
            .apply(crate::formulas::rotate_x(-eps).apply(native))
    })
}

/// Lunar illumination for positions in the common barycentric J2000 frame, seen from Earth's centre.
pub fn compute_geocentric_moon_illumination(moon_position: Vector3, sun_position: Vector3, earth_position: Vector3) -> MoonIllumination {
    compute_moon_illumination(moon_position - earth_position, sun_position - earth_position, crate::formulas::j2000_ecliptic_north())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeus_47a_geometric_example() {
        let (l, b, r) = compute_lunar_ecliptic(2448724.5);
        assert!((l.to_degrees() - 133.162655).abs() < 0.000001);
        assert!((b.to_degrees() + 3.229126).abs() < 0.000001);
        assert!((r - 368409.7).abs() < 0.1);
    }

    #[test]
    fn current_illumination_preserves_original_new_and_full_moon_reference_dates() {
        use crate::formulas::{BodyId, compute_barycentric_position};
        // Original reference dates and precise new/full Moon instants remain covered by the active model.
        for (tt, expected) in [
            (2451550.1, MoonPhase::New), (2460645.5, MoonPhase::New),
            (2459242.5, MoonPhase::Full), (2466447.5, MoonPhase::Full),
            (2451550.2597, MoonPhase::New), (2459243.3028, MoonPhase::Full),
            (2460645.7646, MoonPhase::New),
        ] {
            let moon = evaluate_moon(tt).position;
            let sun = compute_barycentric_position(BodyId::Sun, tt) - compute_barycentric_position(BodyId::Earth, tt);
            let illumination = compute_moon_illumination(moon, sun, crate::formulas::j2000_ecliptic_north());
            assert_eq!(name_moon_phase(illumination), expected, "TT {tt}");
        }
    }

    #[test]
    fn moon_age_to_phase_covers_all_phases() {
        let cases = [
            (0.0, MoonPhase::New),
            (0.1, MoonPhase::WaxingCrescent),
            (0.25, MoonPhase::FirstQuarter),
            (0.4, MoonPhase::WaxingGibbous),
            (0.5, MoonPhase::Full),
            (0.6, MoonPhase::WaningGibbous),
            (0.75, MoonPhase::LastQuarter),
            (0.9, MoonPhase::WaningCrescent),
            (0.98, MoonPhase::New),
        ];
        for (age, phase) in cases {
            assert_eq!(moon_age_to_phase(age), phase, "age {age}");
        }
    }

    #[test]
    fn moon_phase_names() {
        assert_eq!(MoonPhase::WaxingGibbous.name(), "Waxing Gibbous");
    }
}
