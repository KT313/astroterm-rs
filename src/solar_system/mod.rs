//! Step 1 of the frame: `simulate_solar_system`.
//!
//! Inputs: the body tables and the simulated time. Output: where the Sun, the moons and the planets were when the
//! light now reaching the Earth left them, plus the Earth's own state at reception, all barycentric J2000
//! equatorial in AU and AU/day. Runs every frame and evaluates the ephemerides directly (no samples, no cache):
//! three position evaluations per body for the two light-time iterations, about 4 ms in all in release builds
//! (`dev/pipeline-restructuring-results/7/steps-1-2.md`). Light time is iterated against the geocentre; `moon_position` moves a
//! moon by the few milliseconds the observer's site changes its light time, which matters only for the Moon.
use crate::time::StepTimes;
use crate::astro::models::orientation::j2000_ecliptic_north;
use crate::astro::models::{BodyId, BodyState, moons::evaluate_moon, planets::{compute_barycentric_position, evaluate_planet}};
use crate::astro::{AstronomicalTime, LIGHT_SPEED_AU_DAY, MoonIllumination, Vector3, compute_moon_illumination};
use crate::model::Data;

/// The solar system as its light arrives at the Earth at one simulated time.
#[derive(Clone, Debug, PartialEq)]
pub struct SolarSystem {
    pub time_ms: i64,
    pub earth: BodyState,                   // at reception
    pub sun: Vector3,                       // at emission
    pub moons: Vec<BodyState>,              // at emission, one per moon-table entry; the velocity serves `moon_position`
    pub planets: Vec<Vector3>,              // at emission, one per planet-table entry; the Earth's entry is its reception position
}

/// Evaluate every body of the tables for light arriving at the geocentre at `time`.
pub fn simulate_solar_system(data: &Data, time_ms: i64, times: &mut StepTimes) -> SolarSystem {
    let astronomical_time = AstronomicalTime::from_time_ms(time_ms);
    calculate_solar_system_at_tt(data, time_ms, astronomical_time.tt_jd, times)
}

/// Numerical ephemeris boundary: retain fractional TT for light-time calculations and independent references.
/// The result's application timestamp remains the supplied integer millisecond stamp.
pub fn calculate_solar_system_at_tt(data: &Data, time_ms: i64, tt_jd: f64, times: &mut StepTimes) -> SolarSystem {
    // Evaluate Earth's state at reception before calculating when each body's light was emitted.
    times.measure("Solar system.Earth state");
    let earth = evaluate_planet(BodyId::Earth, tt_jd);
    let emitted = |id: BodyId| emission_position(id, tt_jd, earth.position);

    // Calculate the Sun, moons and planets in separate measured intervals.
    times.measure("Solar system.Sun light time");
    let sun = emitted(data.solar_system_sun.sun.id);

    times.measure("Solar system.Moon light time");
    let moons = data.solar_system_moons.moons.iter().map(|moon| {
        let position = emitted(moon.id);
        BodyState { position, velocity: velocity_at(moon.id, tt_jd, position, earth.position) }
    }).collect();

    times.measure("Solar system.Planet light time");
    let planets = data.solar_system_planets.planets.iter().map(|planet| {
        if planet.id == BodyId::Earth { earth.position } else { emitted(planet.id) }
    }).collect();

    SolarSystem { time_ms, earth, sun, moons, planets }
}

impl SolarSystem {
    /// Where moon `index` was when its light reached an observer at barycentric `observer`: the geocentric emission
    /// position moved back by the light-time difference between the observer and the geocentre (under 0.02 s).
    pub fn moon_position(&self, index: usize, observer: Vector3) -> Vector3 {
        let moon = self.moons[index];
        let difference = ((moon.position - observer).length() - (moon.position - self.earth.position).length()) / LIGHT_SPEED_AU_DAY;
        moon.position - moon.velocity * difference
    }

    /// Illumination of moon `index` as seen from the geocentre (the site changes it by far less than its accuracy).
    pub fn moon_illumination(&self, index: usize) -> MoonIllumination {
        compute_moon_illumination(self.moons[index].position - self.earth.position, self.sun - self.earth.position, j2000_ecliptic_north())
    }
}

/// Where a body was when light now arriving at `receiver` left it: the reception position, then two light-time
/// iterations, as the old pipeline did.
fn emission_position(body: BodyId, tt: f64, receiver: Vector3) -> Vector3 {
    let mut position = barycentric_position(body, tt);
    for _ in 0..2 {
        let emission_tt = tt - (position - receiver).length() / LIGHT_SPEED_AU_DAY;
        position = barycentric_position(body, emission_tt);
    }
    position
}

/// The velocity at the emission time that `emission_position` settled on.
fn velocity_at(body: BodyId, tt: f64, emission_position: Vector3, receiver: Vector3) -> Vector3 {
    barycentric_state(body, tt - (emission_position - receiver).length() / LIGHT_SPEED_AU_DAY).velocity
}

fn barycentric_position(body: BodyId, tt: f64) -> Vector3 {
    match body {
        BodyId::Moon => evaluate_moon(tt).position + compute_barycentric_position(BodyId::Earth, tt),
        planet => compute_barycentric_position(planet, tt),
    }
}

fn barycentric_state(body: BodyId, tt: f64) -> BodyState {
    match body {
        BodyId::Moon => evaluate_moon(tt).add_parent(evaluate_planet(BodyId::Earth, tt)),
        planet => evaluate_planet(planet, tt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::J2000;

    fn at_j2000() -> SolarSystem { simulate_solar_system(&Data::default(), crate::astro::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default()) }

    #[test]
    fn bodies_are_taken_at_their_emission_times() {
        let system = at_j2000();
        let tt = crate::astro::AstronomicalTime::from_time_ms(system.time_ms).tt_jd;
        assert_eq!(system.planets.len(), 8);
        assert_eq!(system.moons.len(), 1);
        assert_eq!(system.planets[2], system.earth.position);                            // the Earth entry is the receiver itself
        let light_time = (system.sun - system.earth.position).length() / LIGHT_SPEED_AU_DAY;
        assert!((light_time * 1440.0 - 8.3).abs() < 0.2, "{} min", light_time * 1440.0);  // sunlight takes about 8.3 minutes
        let sun_now = compute_barycentric_position(BodyId::Sun, tt);
        let sun_then = compute_barycentric_position(BodyId::Sun, tt - light_time);
        assert!((system.sun - sun_then).length() < 1e-12 && (system.sun - sun_now).length() > 1e-9);
        for (index, planet) in Data::default().solar_system_planets.planets.iter().enumerate().filter(|(_, planet)| planet.id != BodyId::Earth) {
            let light_time = (system.planets[index] - system.earth.position).length() / LIGHT_SPEED_AU_DAY;
            let then = compute_barycentric_position(planet.id, tt - light_time);
            assert!((system.planets[index] - then).length() < 1e-10, "{}", planet.name);
        }
    }

    #[test]
    fn the_moon_is_geocentric_at_the_geocentre_and_shifts_at_a_site() {
        let system = at_j2000();
        let moon = system.moons[0];
        assert_eq!(system.moon_position(0, system.earth.position), moon.position);
        let distance = (moon.position - system.earth.position).length() * 149_597_870.7;
        assert!((380_000.0..=410_000.0).contains(&distance), "{distance} km");
        let site = system.earth.position + (moon.position - system.earth.position).normalized() * 4.26e-5; // a site right under the Moon, one Earth radius out
        let shifted = system.moon_position(0, site);
        let moved = (shifted - moon.position).length();
        let expected = moon.velocity.length() * 4.26e-5 / LIGHT_SPEED_AU_DAY;                 // one Earth radius less light time
        assert!((moved - expected).abs() < 1e-15 && moved > 0.0);
    }

    #[test]
    fn illumination_follows_the_sun_moon_geometry() {
        let system = at_j2000();
        let illumination = system.moon_illumination(0);
        assert!((illumination.illuminated_fraction - 0.23).abs() < 0.01);                 // 2000-01-01 12:00 TT, Horizons: 23.0 %
        assert!(!illumination.waxing);                                                    // five days before new moon
    }
}
