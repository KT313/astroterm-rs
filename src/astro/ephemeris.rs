//! Positions of stars, planets and the Moon from catalog data and Keplerian orbital elements.
//!
//! References: Explanatory Supplement to the Astronomical Almanac, ch. 8; NASA JPL "Approximate Positions of the
//! Planets" (<https://ssd.jpl.nasa.gov/planets/approx_pos.html>); Paul Schlyter, "How to compute planetary positions"
//! (<https://stjarnhimlen.se/comp/ppcomp.html>).

use std::f64::consts::{PI, TAU};

use super::{Equatorial, J2000, Vector3};

const TO_RAD: f64 = PI / 180.0;

/// Obliquity of the ecliptic at J2000 in radians.
const OBLIQUITY_J2000: f64 = 84381.448 / 3600.0 * TO_RAD;

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

/// Extra mean anomaly terms for Jupiter through Neptune (JPL approximate positions, table 2b).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerturbationTerms {
    pub b: f64,
    pub c: f64,
    pub s: f64,
    pub f: f64,
}

/// Heliocentric orbit of a planet. Rates are per Julian century since J2000.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetOrbit {
    pub elements: OrbitalElements,
    pub rates: OrbitalElements,
    pub perturbations: Option<PerturbationTerms>,
}

/// Geocentric orbit of the Moon. Rates are per day since 1999-12-31T00:00 (Schlyter's epoch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonOrbit {
    pub elements: OrbitalElements,
    pub rates: OrbitalElements,
}

/// The eight named phases of the Moon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoonPhase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    /// All phases, from New Moon through the waxing and waning phases.
    pub const ALL: [MoonPhase; 8] = [
        MoonPhase::New,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::Full,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    /// Human readable name, e.g. "Waxing Crescent".
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 8] = [
            "New Moon",
            "Waxing Crescent",
            "First Quarter",
            "Waxing Gibbous",
            "Full Moon",
            "Waning Gibbous",
            "Last Quarter",
            "Waning Crescent",
        ];
        NAMES[self as usize]
    }
}

/// Apply proper motion (radians per year) to a J2000 catalog position. The result is still in the J2000 frame.
pub fn compute_star_position(catalog: Equatorial, proper_motion: Equatorial, julian_date: f64) -> Equatorial {
    let years_since_j2000 = (julian_date - J2000) / 365.2425;
    Equatorial {
        right_ascension: catalog.right_ascension + proper_motion.right_ascension * years_since_j2000,
        declination: catalog.declination + proper_motion.declination * years_since_j2000,
    }
}

/// Heliocentric position of a planet in rectangular J2000 equatorial coordinates (AU).
///
/// Follows the Explanatory Supplement to the Astronomical Almanac, ch. 8, p. 340.
pub fn compute_planet_heliocentric(orbit: &PlanetOrbit, julian_date: f64) -> Vector3 {
    // 1. propagate the elements to the date
    let centuries = (julian_date - J2000) / 36525.0;
    let elements = propagate_elements(&orbit.elements, &orbit.rates, centuries);
    let OrbitalElements {
        semi_major_axis,
        eccentricity,
        ..
    } = elements;
    let mean_longitude = elements.mean_anomaly + elements.argument_of_periapsis + elements.ascending_node;
    let periapsis_longitude = elements.argument_of_periapsis + elements.ascending_node;

    // 2. mean anomaly, including the extra terms of the outer planets
    let mut mean_anomaly = elements.mean_anomaly;
    if let Some(PerturbationTerms { b, c, s, f }) = orbit.perturbations {
        let ft = f * centuries * TO_RAD;
        mean_anomaly = mean_longitude - periapsis_longitude + b * centuries * centuries + c * ft.cos() + s * ft.sin();
    }

    // 3. solve Kepler's equation
    let mean_anomaly = wrap_degrees_signed(mean_anomaly);
    let initial_guess = mean_anomaly + 180.0 / PI * eccentricity * (mean_anomaly * TO_RAD).sin();
    let eccentric_anomaly = solve_eccentric_anomaly(mean_anomaly, eccentricity, initial_guess);

    // 4. & 5. & 6. position in the orbital plane, rotated to ecliptic then equatorial coordinates
    let (xp, yp) = compute_orbital_plane_position(semi_major_axis, eccentricity, eccentric_anomaly);
    ecliptic_to_equatorial(rotate_orbital_plane_to_ecliptic(xp, yp, &elements))
}

/// Geocentric position of the Moon in rectangular equatorial coordinates (Earth radii).
///
/// Paul Schlyter's method (<https://stjarnhimlen.se/comp/ppcomp.html#6>), including his perturbation terms, which
/// bring the error down from several degrees to a few arcminutes.
pub fn compute_moon_geocentric(orbit: &MoonOrbit, julian_date: f64) -> Vector3 {
    // propagate the elements to the date
    let days = julian_date - 2451543.5; // Schlyter's day 0 is 1999-12-31T00:00
    let elements = propagate_elements(&orbit.elements, &orbit.rates, days);
    let OrbitalElements {
        semi_major_axis,
        eccentricity,
        ..
    } = elements;

    // solve Kepler's equation
    let mean_anomaly = wrap_degrees_signed(elements.mean_anomaly);
    let m = mean_anomaly * TO_RAD;
    let initial_guess = mean_anomaly + 180.0 / PI * eccentricity * m.sin() * (1.0 + eccentricity * m.cos());
    let eccentric_anomaly = solve_eccentric_anomaly(mean_anomaly, eccentricity, initial_guess);

    // unperturbed position in the orbital plane, rotated to ecliptic coordinates
    let (xp, yp) = compute_orbital_plane_position(semi_major_axis, eccentricity, eccentric_anomaly);
    let ecliptic = rotate_orbital_plane_to_ecliptic(xp, yp, &elements);

    // add the perturbations by the Sun, then rotate to equatorial coordinates
    let perturbed = apply_lunar_perturbations(ecliptic, &elements, days);
    ecliptic_to_equatorial(perturbed)
}

/// Age of the Moon within the synodic month in [0, 1): 0 is a New Moon and 0.5 a Full Moon. It is the elongation of
/// the Moon from the Sun along the ecliptic, as a fraction of a full turn, given their geocentric equatorial positions.
pub fn compute_moon_age(moon_geocentric: Vector3, sun_geocentric: Vector3) -> f64 {
    let elongation =
        equatorial_to_ecliptic_longitude(moon_geocentric) - equatorial_to_ecliptic_longitude(sun_geocentric);
    (elongation / TAU).rem_euclid(1.0)
}

/// Named phase for a Moon age in [0, 1).
pub fn moon_age_to_phase(age: f64) -> MoonPhase {
    const UPPER_BOUNDS: [f64; 7] = [0.03, 0.25, 0.27, 0.50, 0.53, 0.75, 0.77];
    if !(0.03..=0.97).contains(&age) {
        return MoonPhase::New;
    }
    let index = UPPER_BOUNDS.iter().position(|&bound| age < bound).unwrap_or(7);
    MoonPhase::ALL[index]
}

/// Add the largest periodic perturbations of the Moon's longitude, latitude and distance (Schlyter, section 9) to an
/// unperturbed ecliptic position. `days` counts from Schlyter's epoch.
fn apply_lunar_perturbations(ecliptic: Vector3, elements: &OrbitalElements, days: f64) -> Vector3 {
    // fundamental arguments in degrees: mean anomalies, mean elongation and argument of latitude
    let sun_mean_anomaly = 356.0470 + 0.9856002585 * days;
    let sun_mean_longitude = sun_mean_anomaly + 282.9404 + 4.70935e-5 * days;
    let moon_mean_anomaly = elements.mean_anomaly;
    let moon_mean_longitude = moon_mean_anomaly + elements.argument_of_periapsis + elements.ascending_node;
    let elongation = moon_mean_longitude - sun_mean_longitude;
    let latitude_argument = moon_mean_longitude - elements.ascending_node;
    let (ms, mm, d, f) = (
        sun_mean_anomaly * TO_RAD,
        moon_mean_anomaly * TO_RAD,
        elongation * TO_RAD,
        latitude_argument * TO_RAD,
    );

    // perturbations: longitude and latitude in degrees, distance in Earth radii
    let longitude_terms = -1.274 * (mm - 2.0 * d).sin() // evection
        + 0.658 * (2.0 * d).sin() // variation
        - 0.186 * ms.sin() // yearly equation
        - 0.059 * (2.0 * mm - 2.0 * d).sin()
        - 0.057 * (mm - 2.0 * d + ms).sin()
        + 0.053 * (mm + 2.0 * d).sin()
        + 0.046 * (2.0 * d - ms).sin()
        + 0.041 * (mm - ms).sin()
        - 0.035 * d.sin() // parallactic equation
        - 0.031 * (mm + ms).sin()
        - 0.015 * (2.0 * f - 2.0 * d).sin()
        + 0.011 * (mm - 4.0 * d).sin();
    let latitude_terms =
        -0.173 * (f - 2.0 * d).sin() - 0.055 * (mm - f - 2.0 * d).sin() - 0.046 * (mm + f - 2.0 * d).sin()
            + 0.033 * (f + 2.0 * d).sin()
            + 0.017 * (2.0 * mm + f).sin();
    let distance_terms = -0.58 * (mm - 2.0 * d).cos() - 0.46 * (2.0 * d).cos();

    // apply them in spherical ecliptic coordinates
    let distance = (ecliptic.x * ecliptic.x + ecliptic.y * ecliptic.y + ecliptic.z * ecliptic.z).sqrt();
    let longitude = ecliptic.y.atan2(ecliptic.x) + longitude_terms * TO_RAD;
    let latitude = (ecliptic.z / distance).asin() + latitude_terms * TO_RAD;
    let distance = distance + distance_terms;
    Vector3 {
        x: distance * latitude.cos() * longitude.cos(),
        y: distance * latitude.cos() * longitude.sin(),
        z: distance * latitude.sin(),
    }
}

/// Elements at `time` units after their epoch, given their rates per unit.
fn propagate_elements(elements: &OrbitalElements, rates: &OrbitalElements, time: f64) -> OrbitalElements {
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
fn wrap_degrees_signed(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// Solve Kepler's equation `M = E - e·sin(E)` (degrees) with Newton's method, at most 10 iterations.
fn solve_eccentric_anomaly(mean_anomaly: f64, eccentricity: f64, initial_guess: f64) -> f64 {
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
fn compute_orbital_plane_position(semi_major_axis: f64, eccentricity: f64, eccentric_anomaly: f64) -> (f64, f64) {
    let e = eccentric_anomaly * TO_RAD;
    let xp = semi_major_axis * (e.cos() - eccentricity);
    let yp = semi_major_axis * (1.0 - eccentricity * eccentricity).sqrt() * e.sin();
    (xp, yp)
}

/// Rotate an orbital plane position to ecliptic coordinates.
fn rotate_orbital_plane_to_ecliptic(xp: f64, yp: f64, elements: &OrbitalElements) -> Vector3 {
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
fn ecliptic_to_equatorial(ecliptic: Vector3) -> Vector3 {
    let (sin_eps, cos_eps) = (OBLIQUITY_J2000.sin(), OBLIQUITY_J2000.cos());
    Vector3 {
        x: ecliptic.x,
        y: cos_eps * ecliptic.y - sin_eps * ecliptic.z,
        z: sin_eps * ecliptic.y + cos_eps * ecliptic.z,
    }
}

/// Ecliptic longitude in radians of a position in equatorial coordinates at J2000.
fn equatorial_to_ecliptic_longitude(equatorial: Vector3) -> f64 {
    let (sin_eps, cos_eps) = (OBLIQUITY_J2000.sin(), OBLIQUITY_J2000.cos());
    (cos_eps * equatorial.y + sin_eps * equatorial.z).atan2(equatorial.x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{EARTH_ORBIT, MOON_ORBIT};

    fn circular_distance(a: f64, b: f64) -> f64 {
        let difference = (a - b).abs();
        difference.min(1.0 - difference)
    }

    fn moon_age_at(julian_date: f64) -> f64 {
        let moon = compute_moon_geocentric(&MOON_ORBIT, julian_date);
        let sun = -compute_planet_heliocentric(&EARTH_ORBIT, julian_date);
        compute_moon_age(moon, sun)
    }

    #[test]
    fn compute_moon_age_matches_reference_dates() {
        // the reference dates of the original test suite, at its tolerance
        for (julian_date, expected) in [(2451550.1, 0.0), (2460645.5, 0.0), (2459242.5, 0.5), (2466447.5, 0.5)] {
            assert!(
                circular_distance(moon_age_at(julian_date), expected) < 0.05,
                "jd {julian_date}"
            );
        }

        // exact instants of new and full moons (2000-01-06T18:14Z, 2021-01-28T19:16Z, 2024-12-01T06:21Z), to within
        // a few hours
        for (julian_date, expected) in [(2451550.2597, 0.0), (2459243.3028, 0.5), (2460645.7646, 0.0)] {
            assert!(
                circular_distance(moon_age_at(julian_date), expected) < 0.005,
                "jd {julian_date}"
            );
        }
    }

    #[test]
    fn moon_position_matches_meeus_example() {
        // Meeus, Astronomical Algorithms, example 47.a: 1992-04-12T00:00 TD, λ = 133.162655° and β = -3.229126°
        // (of date, about 0.11° of precession ahead of J2000), at 368409.7 km
        let moon = compute_moon_geocentric(&MOON_ORBIT, 2448724.5);
        let (sin_eps, cos_eps) = (OBLIQUITY_J2000.sin(), OBLIQUITY_J2000.cos());
        let distance = (moon.x * moon.x + moon.y * moon.y + moon.z * moon.z).sqrt();
        let longitude = equatorial_to_ecliptic_longitude(moon).to_degrees().rem_euclid(360.0);
        let latitude = ((-sin_eps * moon.y + cos_eps * moon.z) / distance).asin().to_degrees();

        assert!((longitude - (133.162655 - 0.11)).abs() < 0.1, "longitude {longitude}");
        assert!((latitude + 3.229126).abs() < 0.1, "latitude {latitude}");
        assert!(
            (distance * 6378.14 - 368409.7).abs() < 500.0,
            "distance {distance} Earth radii"
        );
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
