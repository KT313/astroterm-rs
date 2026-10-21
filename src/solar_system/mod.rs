//! Step 1 of the frame: `simulate_solar_system`.
//!
//! Inputs: the body tables and the simulated time. Output: where the Sun, the moons and the planets were when the
//! light now reaching the Earth left them, plus the Earth's own state at reception, all barycentric J2000
//! equatorial in AU and AU/day. Runs every frame and evaluates the ephemerides directly (no samples, no cache):
//! one pass over each body's series gives its position, velocity and acceleration, and the light time moves that
//! motion back without evaluating the series again (`formulas::compute_emission_state`), about 0.2 ms in all in
//! release builds (`dev/performance.md`, 2026-10-17). Light time is iterated against the geocentre; `moon_position`
//! moves a moon by the few milliseconds the observer's site changes its light time, which matters only for the Moon.
use crate::time::StepTimes;
use crate::formulas::{BodyId, BodyState, evaluate_planet, evaluate_body_motion, compute_emission_state, correct_moon_light_time};
#[cfg(test)]
use crate::formulas::compute_barycentric_position;
use crate::formulas::{AstronomicalTime, MoonIllumination, Vector3, compute_geocentric_moon_illumination};
use crate::model::{SunTable, PlanetsTable, MoonsTable};
#[cfg(test)]
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
pub fn simulate_solar_system(sun_table: &SunTable, planets_table: &PlanetsTable, moons_table: &MoonsTable, time_ms: i64, times: &mut StepTimes) -> SolarSystem {
    let astronomical_time = AstronomicalTime::from_time_ms(time_ms);
    calculate_solar_system_at_tt(sun_table, planets_table, moons_table, time_ms, astronomical_time.tt_jd, times)
}

/// Numerical ephemeris boundary: retain fractional TT for light-time calculations and independent references.
/// The result's application timestamp remains the supplied integer millisecond stamp.
pub fn calculate_solar_system_at_tt(sun_table: &SunTable, planets_table: &PlanetsTable, moons_table: &MoonsTable, time_ms: i64, tt_jd: f64, times: &mut StepTimes) -> SolarSystem {
    // Evaluate the Earth's motion at reception once; every light time below is measured against its position,
    // and step 2 takes the same state instead of evaluating the Earth again.
    times.measure("Solar system.Earth state");
    let earth = evaluate_planet(BodyId::Earth, tt_jd);

    // Each other body: one evaluation at reception, then its motion moved back by its light time.
    times.measure("Solar system.Sun light time");
    let sun = compute_emission_state(evaluate_planet(sun_table.sun.id, tt_jd), earth.position).position;

    times.measure("Solar system.Moon light time");
    let moons = moons_table.moons.iter().map(|moon| compute_emission_state(evaluate_body_motion(moon.id, tt_jd, earth), earth.position)).collect();

    times.measure("Solar system.Planet light time");
    let planets = planets_table.planets.iter().map(|planet| {
        if planet.id == BodyId::Earth { earth.position } else { compute_emission_state(evaluate_planet(planet.id, tt_jd), earth.position).position }
    }).collect();

    SolarSystem { time_ms, earth: earth.state(), sun, moons, planets }
}

impl SolarSystem {
    /// Where moon `index` was when its light reached an observer at barycentric `observer`: the geocentric emission
    /// position moved back by the light-time difference between the observer and the geocentre (under 0.02 s).
    pub fn moon_position(&self, index: usize, observer: Vector3) -> Vector3 {
        correct_moon_light_time(self.moons[index], self.earth.position, observer)
    }

    /// Illumination of moon `index` as seen from the geocentre (the site changes it by far less than its accuracy).
    pub fn moon_illumination(&self, index: usize) -> MoonIllumination {
        compute_geocentric_moon_illumination(self.moons[index].position, self.sun, self.earth.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::{J2000, LIGHT_SPEED_AU_DAY};

    fn at_j2000() -> SolarSystem {
        let bodies = Data::default();
        simulate_solar_system(&bodies.solar_system_sun, &bodies.solar_system_planets, &bodies.solar_system_moons, crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default())
    }

    #[test]
    fn bodies_are_taken_at_their_emission_times() {
        let system = at_j2000();
        let tt = crate::formulas::AstronomicalTime::from_time_ms(system.time_ms).tt_jd;
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
