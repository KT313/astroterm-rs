//! Astronomy: time scales, coordinate conversions, the positions of celestial bodies, and their notations.
//!
//! Conventions (all angles in radians unless noted otherwise):
//! - Azimuth is measured East of North, altitude from the horizon towards the zenith.
//! - Right ascension is measured East of the vernal equinox, declination North of the celestial equator.
//! - Longitude is positive East of the prime meridian, latitude positive North of the equator.
//!
//! # Current model conventions
//!
//! Physical calculations, motion bounds and current magnitudes use f64 (source catalog encodings/colors retain their original precision). TT is currently approximated
//! by UT1, itself approximated by UTC/UT input. `astro/time.rs` documents the calendar and time-scale contract.
//!
//! | Quantity / owner | Native origin and frame | Units / epoch | Current corrections and stage |
//! |---|---|---|---|
//! | Catalog stars / `models::stars` | Barycentric direction, J2000 mean equator/equinox | Unit direction, velocity/distance per Julian year, optional distance in pc at J2000 | Normalized straight-line motion, distance-dependent magnitude, precession and observation |
//! | Planets and Sun / `models::planets` | Heliocentric equatorial J2000 | AU; TT Julian centuries from J2000 | Earth subtraction, precession, horizon rotation; no light-time or aberration |
//! | Moon / `models::moons` | Geocentric mean-of-date ecliptic, converted with fixed J2000 obliquity (approximation) | Earth radii; TT days from JD 2451543.5 | Solar perturbations; inverse source precession to J2000, Earth-relative AU sample; observer rotation and altitude parallax |
//! | Precession and Earth rotation / `models::orientation` | Mean J2000 to mean-of-date, then local horizon | Radians; precession TT, ERA UT1 | Cached C = R3(−EO) P; exact per-frame ERA, then site horizon rotation |
//! | Horizontal / observation | Observer sky: East, North, Up unit vectors | Cartesian unit vectors (radians only at display/API boundaries) | Optional refraction applied once after position update |
//!
//! The current positions are approximations to apparent places, not complete apparent-place solutions. Geometric
//! means before light-time/aberration; geocentric is relative to Earth's center; topocentric is relative to a surface
//! site; refracted includes the atmosphere. Coordinate origin and apparent/geometric status are separate properties.
//! Family adapters declare both, their dependencies and error/precision policy; see [`crate::sky`].

pub mod accuracy;
mod coords;
mod matrix;
pub mod models;
mod notation;
mod orbital;
mod time;

use std::f64::consts::TAU;

pub use accuracy::{
    COMPUTATIONAL_INTERVAL, JULIAN_YEAR_DAYS, JulianDateInterval, MOON_VALIDATED_INTERVAL, ObjectClass,
    PLANET_VALIDATED_INTERVAL, PRECESSION_TARGET_ARCSECONDS, STAR_VALIDATED_INTERVAL, accuracy_target_arcseconds,
};
pub use coords::{
    apply_refraction, correct_for_parallax, equatorial_to_horizontal, horizontal_to_spherical, offset_towards,
    offset_vector_towards, rectangular_to_equatorial, refract_direction,
};
pub use matrix::Matrix3;
pub use models::moons::{MoonOrbit, MoonPhase, compute_moon_age, compute_moon_geocentric, moon_age_to_phase};
pub use models::orientation::{PrecessionMatrix, compute_precession_matrix};
pub use models::planets::{PerturbationTerms, PlanetOrbit, compute_planet_heliocentric};
pub use models::stars::compute_star_position;
pub use notation::{DegreesMinutesSeconds, ElapsedTime, ZodiacSign, azimuth_to_compass, compass_point_to_azimuth};
pub use orbital::OrbitalElements;
pub use time::{
    J2000, SimulationClock, current_julian_date, datetime_to_julian_date, earth_rotation_angle,
    greenwich_mean_sidereal_time, julian_date_to_utc, parse_utc_datetime,
};

/// Position on the local sky of an observer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Horizontal {
    pub azimuth: f64,
    pub altitude: f64,
}

/// Position on the celestial sphere.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Equatorial {
    pub right_ascension: f64,
    pub declination: f64,
}

/// Geographic location of the observer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observer {
    pub latitude: f64,
    pub longitude: f64,
}

/// Rectangular coordinates, e.g. heliocentric or geocentric equatorial coordinates in AU.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Horizontal {
    /// Unit vector pointing at this position, with x East, y North and z up.
    pub fn to_unit_vector(self) -> Vector3 {
        let (sin_alt, cos_alt) = self.altitude.sin_cos();
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        Vector3 {
            x: cos_alt * sin_az,
            y: cos_alt * cos_az,
            z: sin_alt,
        }
    }

    /// Position a vector (x East, y North, z up) points at.
    pub fn from_vector(vector: Vector3) -> Horizontal {
        let altitude = (vector.z / vector.length()).clamp(-1.0, 1.0).asin();
        Horizontal {
            azimuth: vector.x.atan2(vector.y).rem_euclid(TAU),
            altitude,
        }
    }
}

impl Equatorial {
    /// Unit vector pointing at this position, with x towards the equinox, y at right ascension 90° and z North.
    pub fn to_unit_vector(self) -> Vector3 {
        let (sin_dec, cos_dec) = self.declination.sin_cos();
        let (sin_ra, cos_ra) = self.right_ascension.sin_cos();
        Vector3 {
            x: cos_dec * cos_ra,
            y: cos_dec * sin_ra,
            z: sin_dec,
        }
    }
}

impl Vector3 {
    pub fn normalized(self) -> Self {
        self * (1.0 / self.x.hypot(self.y).hypot(self.z))
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn cross(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn dot(self, other: Vector3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
}

impl std::ops::Add for Vector3 {
    type Output = Vector3;

    fn add(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

impl std::ops::Mul<f64> for Vector3 {
    type Output = Vector3;

    fn mul(self, factor: f64) -> Vector3 {
        Vector3 {
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }
}

impl std::ops::Sub for Vector3 {
    type Output = Vector3;

    fn sub(self, other: Vector3) -> Vector3 {
        Vector3 {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }
}

impl std::ops::Neg for Vector3 {
    type Output = Vector3;

    fn neg(self) -> Vector3 {
        Vector3 {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

/// Normalize an angle in radians to [0, 2π).
pub fn normalize_radians(angle: f64) -> f64 {
    angle.rem_euclid(TAU)
}

/// Linearly map `input` from [min_float, max_float] to the integer range [min_int, max_int], rounding to nearest.
pub fn map_float_to_int_range(min_float: f64, max_float: f64, min_int: i32, max_int: i32, input: f64) -> i32 {
    let percent = (input - min_float) / (max_float - min_float);
    min_int + (f64::from(max_int - min_int) * percent).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn normalized_finite_angles_stay_in_one_turn(angle in -1e12_f64..1e12_f64) {
            let wrapped = normalize_radians(angle);
            prop_assert!((0.0..TAU).contains(&wrapped));
            prop_assert_eq!(normalize_radians(wrapped), wrapped);
        }
    }

    #[test]
    fn normalize_radians_wraps_into_one_turn() {
        assert!((normalize_radians(-0.5) - (TAU - 0.5)).abs() < 1e-12);
        assert!((normalize_radians(TAU + 0.25) - 0.25).abs() < 1e-12);
        assert_eq!(normalize_radians(0.0), 0.0);
    }

    #[test]
    fn map_float_to_int_range_matches_reference() {
        assert_eq!(map_float_to_int_range(0.0, 1.0, 0, 100, 0.5), 50);
        assert_eq!(map_float_to_int_range(-1.0, 1.0, 0, 10, 0.0), 5);
        assert_eq!(map_float_to_int_range(0.0, 10.0, 0, 100, 7.5), 75);
    }
}
