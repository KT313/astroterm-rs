//! JPL approximate Kepler ephemeris, geometric heliocentric equatorial J2000, AU, TT, f64.
//! No observer dependencies. Sun at origin approximates the future barycentric frame. Elements valid 1800–2050;
//! extrapolation outside that range does not imply astronomical accuracy. Cache policy is in sky::simulation.
use crate::astro::orbital::*;
use crate::astro::{J2000, Vector3};
use std::f64::consts::PI;
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

/// Heliocentric position of a planet in rectangular J2000 equatorial coordinates (AU).
/// Time is TT. The result is geometric: no observer subtraction, light-time or aberration.
///
/// Follows the Explanatory Supplement to the Astronomical Almanac, ch. 8, p. 340.
pub fn compute_planet_heliocentric(orbit: &PlanetOrbit, julian_date_tt: f64) -> Vector3 {
    // 1. propagate the elements to the date
    let centuries = (julian_date_tt - J2000) / 36525.0;
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

const fn perturbations(b: f64, c: f64, s: f64, f: f64) -> Option<PerturbationTerms> {
    Some(PerturbationTerms { b, c, s, f })
}

pub const MERCURY_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        0.38709843,
        0.20563661,
        7.00559432,
        174.79394829,
        29.11810076,
        48.33961819,
    ),
    rates: elements(
        0.00000000,
        0.00002123,
        -0.00590158,
        149472.51546610,
        0.28154195,
        -0.12214182,
    ),
    perturbations: None,
};

pub const VENUS_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        0.72332102,
        0.00676399,
        3.39777545,
        50.21215137,
        55.09494217,
        76.67261496,
    ),
    rates: elements(
        -0.00000026,
        -0.00005107,
        0.00043494,
        58517.75880612,
        0.32953822,
        -0.27274174,
    ),
    perturbations: None,
};

/// The Earth-Moon barycenter.
pub const EARTH_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        1.00000018,
        0.01673163,
        -0.00054346,
        -2.46314313,
        108.04266274,
        -5.11260389,
    ),
    rates: elements(
        -0.00000003,
        -0.00003661,
        -0.01337178,
        35999.05511069,
        0.55919116,
        -0.24123856,
    ),
    perturbations: None,
};

pub const MARS_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        1.52371243,
        0.09336511,
        1.85181869,
        19.34931620,
        -73.63065768,
        49.71320984,
    ),
    rates: elements(
        0.00000097,
        0.00009149,
        -0.00724757,
        19139.84710618,
        0.72076056,
        -0.26852431,
    ),
    perturbations: None,
};

pub const JUPITER_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        5.20248019,
        0.04853590,
        1.29861416,
        20.05983908,
        -86.01787410,
        100.29282654,
    ),
    rates: elements(
        -0.00002864,
        0.00018026,
        -0.00322699,
        3034.72172561,
        0.05174577,
        0.13024619,
    ),
    perturbations: perturbations(-0.00012452, 0.06064060, -0.35635438, 38.35125000),
};

pub const SATURN_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        9.54149883,
        0.05550825,
        2.49424102,
        -42.78564734,
        -20.77862639,
        113.63998702,
    ),
    rates: elements(
        -0.00003065,
        -0.00032044,
        0.00451969,
        1221.57315246,
        0.79194480,
        -0.25015002,
    ),
    perturbations: perturbations(0.00025899, -0.13434469, 0.87320147, 38.35125000),
};

pub const URANUS_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        19.18797948,
        0.04685740,
        0.77298127,
        141.76872184,
        98.47154226,
        73.96250215,
    ),
    rates: elements(
        -0.00020455,
        -0.00001550,
        -0.00180155,
        428.40245610,
        0.03527286,
        0.05739699,
    ),
    perturbations: perturbations(0.00058331, -0.97731848, 0.17689245, 7.67025000),
};

pub const NEPTUNE_ORBIT: PlanetOrbit = PlanetOrbit {
    elements: elements(
        30.06952752,
        0.00895439,
        1.77005520,
        257.54130563,
        -85.10477129,
        131.78635853,
    ),
    rates: elements(
        0.00006447,
        0.00000818,
        0.00022400,
        218.45505376,
        0.01616240,
        -0.00606302,
    ),
    perturbations: perturbations(-0.00041348, 0.68346318, -0.10162547, 7.67025000),
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circular_orbit_has_known_j2000_equatorial_positions() {
        let orbit = PlanetOrbit {
            elements: elements(1.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            rates: elements(0.0, 0.0, 0.0, 36000.0, 0.0, 0.0),
            perturbations: None,
        };
        let start = compute_planet_heliocentric(&orbit, J2000);
        let quarter = compute_planet_heliocentric(&orbit, J2000 + 365.25 / 4.0);
        assert!((start - Vector3 { x: 1.0, y: 0.0, z: 0.0 }).length() < 1e-12);
        assert!(
            (quarter
                - Vector3 {
                    x: 0.0,
                    y: OBLIQUITY_J2000.cos(),
                    z: OBLIQUITY_J2000.sin()
                })
            .length()
                < 1e-12
        );
    }
    #[test]
    fn earth_heliocentric_position_matches_independent_horizons_near_today() {
        let references: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/reference/references.json")).unwrap();
        let earth = &references["barycentric_vectors"]["earth"]["rows"][0];
        let sun = &references["barycentric_vectors"]["sun"]["rows"][0];
        let tt = earth["jd_tdb"].as_f64().unwrap(); // TT≈TDB, as documented by the reference generator
        let vector = |row: &serde_json::Value| Vector3 {
            x: row["position_au"][0].as_f64().unwrap(),
            y: row["position_au"][1].as_f64().unwrap(),
            z: row["position_au"][2].as_f64().unwrap(),
        };
        let error = (compute_planet_heliocentric(&EARTH_ORBIT, tt) - (vector(earth) - vector(sun))).length();
        assert!(error < 0.0001, "Earth position error {error} AU");
    }
}
