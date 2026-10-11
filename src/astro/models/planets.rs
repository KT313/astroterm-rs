//! VSOP87E geometric barycentric ecliptic J2000 states, rotated to equatorial J2000, AU and AU/day, TT≈TDB.
//! Bretagnon & Francou (1988). Earth is the geocenter in this variant, not the Earth–Moon barycenter.
//! Observer corrections belong to sky::observation. Physical accuracy is qualified separately from cache error.
mod legacy;
use super::{BodyId, BodyState};
use crate::astro::{Vector3, orbital::ecliptic_to_equatorial};
pub use legacy::*; // original C fixture APIs; not used by the production ephemeris

/// Evaluate one barycentric position at TT (the periodic TDB−TT difference is neglected).
pub fn compute_barycentric_position(body: BodyId, tt: f64) -> Vector3 {
    use vsop87::vsop87e as e;
    let p = match body {
        BodyId::Sun => e::sun(tt),
        BodyId::Mercury => e::mercury(tt),
        BodyId::Venus => e::venus(tt),
        BodyId::Earth => e::earth(tt),
        BodyId::Mars => e::mars(tt),
        BodyId::Jupiter => e::jupiter(tt),
        BodyId::Saturn => e::saturn(tt),
        BodyId::Uranus => e::uranus(tt),
        BodyId::Neptune => e::neptune(tt),
        BodyId::Moon => panic!("Moon belongs to the lunar family"),
    };
    ecliptic_to_equatorial(Vector3 { x: p.x, y: p.y, z: p.z })
}

/// One body's barycentric state at one TT epoch: three position evaluations for the finite-difference velocity.
pub fn evaluate_planet(body: BodyId, tt: f64) -> BodyState {
    super::state::evaluate_with_velocity(tt, |t| compute_barycentric_position(body, t))
}

/// Planetary batch, including the moving Sun and Earth, at one TT epoch.
pub fn evaluate_planets(tt: f64) -> [BodyState; 9] {
    BodyId::PLANETS.map(|body| evaluate_planet(body, tt))
}
