//! Apparent positions of the objects for a given time and observer.

use crate::astro::{
    Observer, compute_moon_age, compute_moon_geocentric, compute_planet_heliocentric, compute_star_position,
    correct_for_parallax, equatorial_to_horizontal, moon_age_to_phase, rectangular_to_equatorial,
};
use crate::catalog::EARTH_ORBIT;

use super::{Moon, Planet, Star};

/// Move every star to its apparent position, including proper motion since J2000.
pub fn update_star_positions(stars: &mut [Star], julian_date: f64, sidereal_time: f64, observer: &Observer) {
    for star in stars {
        let equatorial = compute_star_position(star.catalog_position, star.proper_motion, julian_date);
        star.position = equatorial_to_horizontal(equatorial, sidereal_time, observer);
    }
}

/// Move the Sun and the planets to their apparent positions, seen from the Earth.
pub fn update_planet_positions(planets: &mut [Planet], julian_date: f64, sidereal_time: f64, observer: &Observer) {
    let earth = compute_planet_heliocentric(&EARTH_ORBIT, julian_date);
    for planet in planets {
        let geocentric = match planet.orbit {
            Some(orbit) => compute_planet_heliocentric(orbit, julian_date) - earth,
            None => -earth, // the Sun is (roughly) the origin of the heliocentric frame
        };
        planet.position = equatorial_to_horizontal(rectangular_to_equatorial(geocentric), sidereal_time, observer);
    }
}

/// Move the Moon to its apparent position, as seen from the Earth's surface, and update its phase.
pub fn update_moon(moon: &mut Moon, julian_date: f64, sidereal_time: f64, observer: &Observer) {
    // position, corrected for the observer being on the surface rather than at the center of the Earth
    let geocentric = compute_moon_geocentric(moon.orbit, julian_date);
    let position = equatorial_to_horizontal(rectangular_to_equatorial(geocentric), sidereal_time, observer);
    moon.position = correct_for_parallax(position, geocentric.length());

    // phase, from the elongation from the Sun
    let sun = -compute_planet_heliocentric(&EARTH_ORBIT, julian_date);
    moon.phase = moon_age_to_phase(compute_moon_age(geocentric, sun));
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::astro::{MoonPhase, greenwich_mean_sidereal_time};
    use crate::catalog::load_embedded_catalog;
    use crate::sky::Sky;

    const STAR_EPSILON: f64 = 0.01;
    const PLANET_EPSILON: f64 = 0.02;
    const MOON_EPSILON: f64 = 0.06;

    /// 2020-10-23T12:00 UT1 in Boston, MA. Reference positions from Stellarium.
    fn update_boston_sky() -> Sky {
        let julian_date = 2459146.0;
        let boston = Observer {
            latitude: 42.3601 * PI / 180.0,
            longitude: -71.0589 * PI / 180.0,
        };
        let sidereal_time = greenwich_mean_sidereal_time(julian_date);

        let mut sky = Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
        update_star_positions(&mut sky.stars, julian_date, sidereal_time, &boston);
        update_planet_positions(&mut sky.planets, julian_date, sidereal_time, &boston);
        update_moon(&mut sky.moon, julian_date, sidereal_time, &boston);
        sky
    }

    fn assert_position(actual: crate::astro::Horizontal, azimuth: f64, altitude: f64, epsilon: f64) {
        assert!(
            (actual.azimuth - azimuth).abs() < epsilon,
            "azimuth {} vs {azimuth}",
            actual.azimuth
        );
        assert!(
            (actual.altitude - altitude).abs() < epsilon,
            "altitude {} vs {altitude}",
            actual.altitude
        );
    }

    #[test]
    fn star_positions_match_reference() {
        let sky = update_boston_sky();
        let vega = &sky.stars[7000];
        assert_eq!((vega.catalog_number, vega.appearance.label), (7001, Some("Vega")));
        assert_position(vega.position, 0.547246, 0.0, STAR_EPSILON);

        let arcturus = &sky.stars[5339];
        assert_eq!(arcturus.appearance.label, Some("Arcturus"));
        assert_position(arcturus.position, 1.511414, 0.440355, STAR_EPSILON);
    }

    #[test]
    fn planet_positions_match_reference() {
        let sky = update_boston_sky();
        let find = |label| {
            sky.planets
                .iter()
                .find(|planet| planet.appearance.label == Some(label))
                .unwrap()
        };
        assert_position(find("Sun").position, 1.993463, 0.145643, STAR_EPSILON);
        assert_position(find("Mars").position, 5.1954878, -0.341956, PLANET_EPSILON);
        assert_position(find("Neptune").position, 5.5390816, -0.779650, PLANET_EPSILON);
    }

    #[test]
    fn moon_position_matches_reference() {
        let sky = update_boston_sky();
        assert_position(sky.moon.position, 0.7817126, -1.118899, MOON_EPSILON);
        let first_quarter_soon = [MoonPhase::WaxingCrescent, MoonPhase::FirstQuarter]; // first quarter was at 13:23
        assert!(first_quarter_soon.contains(&sky.moon.phase), "{:?}", sky.moon.phase);
    }
}
