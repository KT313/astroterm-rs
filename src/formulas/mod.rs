//! Astronomy: time scales, coordinate conversions, the positions of celestial bodies, and their notations.
//!
//! Conventions (all angles in radians unless noted otherwise):
//! - Azimuth is measured East of North, altitude from the horizon towards the zenith.
//! - Right ascension is measured East of the vernal equinox, declination North of the celestial equator.
//! - Longitude is positive East of the prime meridian, latitude positive North of the equator.
//!
//! # Current model conventions
//!
//! Physical calculations use f64; immutable catalog storage uses certified compact representations. Ephemerides,
//! stellar motion and slow orientation use TT; UTC/UT input approximates UT1 and Espenak–Meeus ΔT supplies TT.
//!
//! | Quantity / owner | Native origin and frame | Units / epoch | Corrections and stage |
//! |---|---|---|---|
//! | Catalog stars / `regions/stars.rs` | Barycentric direction, J2000 mean equator/equinox | Unit direction, velocity/distance per Julian year, optional parsec distance at J2000 | Normalized straight-line motion and brightness, then aberration/orientation in observation |
//! | Planets and Sun / `solar_system/planets.rs` | VSOP87E barycentric ecliptic J2000, adapted to equatorial J2000 | AU and AU/day, TT≈TDB | Observation requests emission epochs, subtracts observer, applies aberration and orientation |
//! | Moon / `solar_system/moons.rs` | Meeus/ELP geometric mean ecliptic of date | Radians/km, TT; adapter returns Earth-relative J2000 AU/AU-day | Date obliquity and inverse precession; coordinator adds same-time Earth; shared observer corrections |
//! | Orientation / `observation/orientation.rs` | J2000 to true-of-date, then body-fixed and local horizon | Radians; precession/nutation TT, ERA UT1 | C = R3(−EO) N P, model-consistent equinox; exact per-frame ERA; WGS84 sea-level site |
//! | Horizontal / observation | Observer sky: East, North, Up unit vectors | Cartesian unit vectors | Refraction optional, exactly once after apparent-place corrections |
//!
//! The current positions are approximations to apparent places, with the documented omissions (stellar parallax, light deflection and polar motion). Geometric
//! means before light-time/aberration; geocentric is relative to Earth's center; topocentric is relative to a surface
//! site; refracted includes the atmosphere. Coordinate origin and apparent/geometric status are separate properties.
//! Each family declares its coordinates, units and approximation policy.

// This module imports only its own code, the standard library and external numerical libraries.
mod types;
mod math;
mod constants;
mod accuracy;
mod time_scales;
mod solar_system;
mod observation;
mod regions;

pub use types::*;
pub use math::normalize_radians;
pub use constants::{LIGHT_SPEED_AU_DAY, SINGULAR_RATIO, J2000, JULIAN_YEAR_DAYS, COMPUTATIONAL_INTERVAL,
    STAR_VALIDATED_INTERVAL, PLANET_VALIDATED_INTERVAL, MOON_VALIDATED_INTERVAL, PRECESSION_TARGET_ARCSECONDS, ACCURACY_WARNING};
pub use accuracy::*;
pub use time_scales::*;
pub use solar_system::*;
pub use observation::*;
pub use regions::*;
