//! Light-travel-time corrections in J2000 barycentric coordinates, AU and AU/day.
use crate::formulas::{BodyId, BodyMotion, BodyState, Vector3, LIGHT_SPEED_AU_DAY, evaluate_moon, evaluate_planet};

/// Where a body was, and how fast it moved, when light now arriving at `receiver` left it: the body's motion at
/// reception moved back along its second-order expansion by the light time, which is refined twice from the
/// distance (the old pipeline's two iterations, each of which evaluated the series again). The neglected third
/// derivative amounts to under 2e-11 AU for Mercury seen across the Sun (below 1e-5″); for the Moon, whose own
/// acceleration around the Earth `BodyMotion` omits, the second-order term itself is about 2 mm.
pub fn compute_emission_state(motion: BodyMotion, receiver: Vector3) -> BodyState {
    let mut emitted = motion.state();
    for _ in 0..2 {
        let light_time = (emitted.position - receiver).length() / LIGHT_SPEED_AU_DAY;
        emitted = motion.after(-light_time);
    }
    emitted
}

/// A body's motion at `tt`: a planet or the Sun from its series, the Moon from the lunar model carried along by
/// the Earth's motion `earth` at the same epoch.
pub fn evaluate_body_motion(body: BodyId, tt: f64, earth: BodyMotion) -> BodyMotion {
    match body {
        BodyId::Moon => evaluate_moon(tt).carried_by(earth),
        planet => evaluate_planet(planet, tt),
    }
}

/// Adjust a geocentric lunar sample for the difference in light travel to the observer's site.
pub fn correct_moon_light_time(moon: BodyState, earth_position: Vector3, observer_position: Vector3) -> Vector3 {
    let difference = ((moon.position - observer_position).length() - (moon.position - earth_position).length()) / LIGHT_SPEED_AU_DAY;
    moon.position - moon.velocity * difference
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::{COMPUTATIONAL_INTERVAL, compute_barycentric_position};

    /// Re-evaluate both the body and its parent at the requested date, without extrapolation.
    fn evaluate_direct_body_state(body: BodyId, tt: f64) -> BodyState {
        match body {
            BodyId::Moon => evaluate_moon(tt).add_parent(evaluate_planet(BodyId::Earth, tt).state()),
            planet => evaluate_planet(planet, tt).state(),
        }
    }

    /// Repeat the light-time iterations with full model evaluations at each emission date.
    fn evaluate_direct_emission(body: BodyId, tt: f64, receiver: Vector3) -> BodyState {
        let mut emitted = evaluate_direct_body_state(body, tt);
        for _ in 0..2 {
            let light_time = (emitted.position - receiver).length() / LIGHT_SPEED_AU_DAY;
            emitted = evaluate_direct_body_state(body, tt - light_time);
        }
        emitted
    }

    #[test]
    fn expansion_matches_direct_models_across_the_computational_interval() {
        // Sample the full interval, including its first date and its last representable date.
        let start = COMPUTATIONAL_INTERVAL.start_tt;
        let end = COMPUTATIONAL_INTERVAL.end_tt;
        let bodies = [BodyId::Sun, BodyId::Mercury, BodyId::Venus, BodyId::Mars, BodyId::Jupiter,
            BodyId::Saturn, BodyId::Uranus, BodyId::Neptune, BodyId::Moon];
        for index in 0..=40 {
            let tt = (start + (end - start) * f64::from(index) / 40.0).min(end.next_down());
            assert!(COMPUTATIONAL_INTERVAL.contains(tt));
            let earth = evaluate_planet(BodyId::Earth, tt);

            // Check the approximation against fresh position and velocity calculations.
            for &body in &bodies {
                let motion = evaluate_body_motion(body, tt, earth);
                let emitted = compute_emission_state(motion, earth.position);
                let reference = evaluate_direct_emission(body, tt, earth.position);
                let position_error = (emitted.position - reference.position).length();
                let velocity_error = (emitted.velocity - reference.velocity).length();
                assert!(position_error < 1e-10, "{body:?} TT {tt}: position error {position_error:e} AU");
                assert!(velocity_error < 1e-8, "{body:?} TT {tt}: velocity error {velocity_error:e} AU/day");

                // Angular checks also account for the Moon's much shorter distance to the observer.
                // These regression limits include floating-point emission-date rounding; they do not
                // assert the physical accuracy of the underlying models over this entire interval.
                let direction = (emitted.position - earth.position).normalized();
                let reference_direction = (reference.position - earth.position).normalized();
                let angular_error = direction.cross(reference_direction).length()
                    .atan2(direction.dot(reference_direction)).to_degrees() * 3600.0;
                let angular_limit = if body == BodyId::Moon { 0.005 } else { 0.0001 };
                assert!(angular_error < angular_limit, "{body:?} TT {tt}: angular error {angular_error:e} arcseconds");
            }
        }
    }

    #[test]
    fn the_expansion_lands_where_the_series_was_at_the_emission_time() {
        for body in [BodyId::Mercury, BodyId::Mars, BodyId::Neptune, BodyId::Sun] {
            for tt in [2451545.0, 2461041.5, 2440000.0] {
                let earth = evaluate_planet(BodyId::Earth, tt);
                let emitted = compute_emission_state(evaluate_planet(body, tt), earth.position);
                let light_time = (emitted.position - earth.position).length() / LIGHT_SPEED_AU_DAY;
                let then = evaluate_planet(body, tt - light_time);
                let error = (emitted.position - then.position).length();
                assert!(error < 1e-10, "{body:?} {tt}: {error:e} AU");                   // the omitted third derivative
                let velocity_error = (emitted.velocity - then.velocity).length();
                assert!(velocity_error < 1e-8, "{body:?} {tt}: {velocity_error:e} AU/day"); // the velocity is first order: ½·jerk·τ², 5e-9 for Mercury
                assert!(light_time > 0.002 && light_time < 0.2, "{light_time}");         // 3 minutes (Mars at opposition) to 4.3 hours (Neptune)
            }
        }
    }

    #[test]
    fn the_moon_rides_on_the_earth_and_planets_ignore_it() {
        let tt = 2451545.0;
        let earth = evaluate_planet(BodyId::Earth, tt);
        let moon = evaluate_body_motion(BodyId::Moon, tt, earth);
        assert_eq!(moon.acceleration, earth.acceleration);
        let distance = (moon.position - earth.position).length() * 149_597_870.7;
        assert!((380_000.0..=410_000.0).contains(&distance), "{distance} km");
        assert_eq!(evaluate_body_motion(BodyId::Mars, tt, earth), evaluate_planet(BodyId::Mars, tt));
        assert_eq!(compute_barycentric_position(BodyId::Mars, tt), evaluate_planet(BodyId::Mars, tt).position);
    }
}
