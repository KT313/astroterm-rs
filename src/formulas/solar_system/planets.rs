//! VSOP87E geometric barycentric ecliptic J2000 states, rotated to equatorial J2000, AU and AU/day, TT≈TDB.
//! Bretagnon & Francou (1988). Earth is the geocenter in this variant, not the Earth–Moon barycenter.
//! Observer corrections live in observer.rs and apparent_position.rs. Physical accuracy is qualified separately from cache error.
use crate::formulas::{BodyId, BodyState};
use crate::formulas::{Vector3, ecliptic_to_equatorial};
use crate::formulas::constants::VELOCITY_SAMPLE_OFFSET_DAYS;
use crate::formulas::math::compute_velocity;

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
    // Evaluate the central position and the two velocity samples in the original order.
    let before_tt = tt - VELOCITY_SAMPLE_OFFSET_DAYS;
    let after_tt = tt + VELOCITY_SAMPLE_OFFSET_DAYS;
    let position = compute_barycentric_position(body, tt);
    let after_position = compute_barycentric_position(body, after_tt);
    let before_position = compute_barycentric_position(body, before_tt);

    // The two samples are separated by one offset before and one offset after the requested date.
    let velocity = compute_velocity(before_position, after_position, VELOCITY_SAMPLE_OFFSET_DAYS + VELOCITY_SAMPLE_OFFSET_DAYS);
    BodyState { position, velocity }
}

/// Planetary batch, including the moving Sun and Earth, at one TT epoch.
pub fn evaluate_planets(tt: f64) -> [BodyState; 9] {
    let mut states = [BodyState::default(); 9];
    for (state, body) in states.iter_mut().zip(BodyId::PLANETS) {
        *state = evaluate_planet(body, tt);
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read_reference_position(row: &serde_json::Value) -> Vector3 {
        Vector3 {
            x: row["position_au"][0].as_f64().unwrap(),
            y: row["position_au"][1].as_f64().unwrap(),
            z: row["position_au"][2].as_f64().unwrap(),
        }
    }

    #[test]
    fn earth_heliocentric_position_matches_independent_horizons_near_today() {
        let references: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/reference/references.json")).unwrap();
        let earth = &references["barycentric_vectors"]["earth"]["rows"][0];
        let sun = &references["barycentric_vectors"]["sun"]["rows"][0];
        let tt = earth["jd_tdb"].as_f64().unwrap(); // TT≈TDB, as documented by the reference generator
        let error = ((compute_barycentric_position(BodyId::Earth, tt) - compute_barycentric_position(BodyId::Sun, tt)) - (read_reference_position(earth) - read_reference_position(sun))).length();
        assert!(error < 0.0001, "Earth position error {error} AU");
    }
}
