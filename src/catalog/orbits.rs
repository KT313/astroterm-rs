//! Compatibility exports for model-owned coefficients; catalog I/O does not own astronomical theories.
pub use crate::astro::models::moons::MOON_ORBIT;
pub use crate::astro::models::planets::{
    EARTH_ORBIT, JUPITER_ORBIT, MARS_ORBIT, MERCURY_ORBIT, NEPTUNE_ORBIT, SATURN_ORBIT, URANUS_ORBIT, VENUS_ORBIT,
};
