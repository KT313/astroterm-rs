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
    use crate::formulas::compute_barycentric_position;

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
