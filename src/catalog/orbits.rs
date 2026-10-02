//! Keplerian orbital elements of the planets and the Moon.
//!
//! Planets: NASA JPL "Approximate Positions of the Planets" (<https://ssd.jpl.nasa.gov/planets/approx_pos.html>),
//! recomputed to use the argument of perihelion and the mean anomaly. Rates are per Julian century.
//!
//! Moon: Paul Schlyter, "How to compute planetary positions" (<https://stjarnhimlen.se/comp/ppcomp.html#4>). Rates
//! are per day.

use crate::astro::{MoonOrbit, OrbitalElements, PerturbationTerms, PlanetOrbit};

const fn elements(a: f64, e: f64, i: f64, m: f64, w: f64, node: f64) -> OrbitalElements {
    OrbitalElements {
        semi_major_axis: a,
        eccentricity: e,
        inclination: i,
        mean_anomaly: m,
        argument_of_periapsis: w,
        ascending_node: node,
    }
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

pub const MOON_ORBIT: MoonOrbit = MoonOrbit {
    elements: elements(60.2666, 0.054900, 5.1454, 115.3654, 318.0634, 125.1228),
    rates: elements(0.0, 0.0, 0.0, 13.0649929509, 0.1643573223, -0.0529538083),
};
