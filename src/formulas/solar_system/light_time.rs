//! Light-travel-time corrections in J2000 barycentric coordinates, AU and AU/day.
use crate::formulas::{BodyId, BodyState, Vector3, LIGHT_SPEED_AU_DAY, evaluate_moon, evaluate_planet, compute_barycentric_position};

/// Where a body was when light now arriving at `receiver` left it: the reception position, then two light-time
/// iterations, as the old pipeline did.
pub fn compute_emission_position(body: BodyId, tt: f64, receiver: Vector3) -> Vector3 {
    let mut position = evaluate_body_position(body, tt);
    for _ in 0..2 {
        let emission_tt = tt - (position - receiver).length() / LIGHT_SPEED_AU_DAY;
        position = evaluate_body_position(body, emission_tt);
    }
    position
}

/// The velocity at the emission time that `compute_emission_position` settled on.
pub fn compute_emission_velocity(body: BodyId, tt: f64, emission_position: Vector3, receiver: Vector3) -> Vector3 {
    evaluate_body_state(body, tt - (emission_position - receiver).length() / LIGHT_SPEED_AU_DAY).velocity
}

fn evaluate_body_position(body: BodyId, tt: f64) -> Vector3 {
    match body {
        BodyId::Moon => evaluate_moon(tt).position + compute_barycentric_position(BodyId::Earth, tt),
        planet => compute_barycentric_position(planet, tt),
    }
}

fn evaluate_body_state(body: BodyId, tt: f64) -> BodyState {
    match body {
        BodyId::Moon => evaluate_moon(tt).add_parent(evaluate_planet(BodyId::Earth, tt)),
        planet => evaluate_planet(planet, tt),
    }
}

/// Adjust a geocentric lunar sample for the difference in light travel to the observer's site.
pub fn correct_moon_light_time(moon: BodyState, earth_position: Vector3, observer_position: Vector3) -> Vector3 {
    let difference = ((moon.position - observer_position).length() - (moon.position - earth_position).length()) / LIGHT_SPEED_AU_DAY;
    moon.position - moon.velocity * difference
}
