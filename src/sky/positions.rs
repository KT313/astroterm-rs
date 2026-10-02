//! Apparent positions of the objects for a given time and observer.

use crate::astro::{
    Observer, PrecessionMatrix, apply_refraction, compute_moon_age, compute_moon_geocentric,
    compute_planet_heliocentric, compute_precession_matrix, compute_star_position, correct_for_parallax,
    equatorial_to_horizontal, greenwich_mean_sidereal_time, moon_age_to_phase, rectangular_to_equatorial,
};
use crate::catalog::EARTH_ORBIT;

use super::{Moon, Planet, Sky, Star};

/// Move every object to its apparent position for the observer at `julian_date`.
pub fn update_sky_positions(sky: &mut Sky, julian_date: f64, observer: &Observer) {
    // Earth's orientation at the date: its rotation, and how far its axis has precessed since J2000
    let sidereal_time = greenwich_mean_sidereal_time(julian_date);
    let precession = compute_precession_matrix(julian_date);

    // positions of all objects
    update_star_positions(&mut sky.stars, julian_date, sidereal_time, &precession, observer);
    update_planet_positions(&mut sky.planets, julian_date, sidereal_time, &precession, observer);
    update_moon(&mut sky.moon, julian_date, sidereal_time, observer);
}

/// Move every star to its apparent position, including proper motion since J2000. `precession` rotates the J2000
/// catalog positions to the date.
fn update_star_positions(
    stars: &mut [Star],
    julian_date: f64,
    sidereal_time: f64,
    precession: &PrecessionMatrix,
    observer: &Observer,
) {
    for star in stars {
        let equatorial = compute_star_position(star.catalog_position, star.proper_motion, julian_date);
        let equatorial = precession.apply_to_equatorial(equatorial);
        star.position = equatorial_to_horizontal(equatorial, sidereal_time, observer);
    }
}

/// Move the Sun and the planets to their apparent positions, seen from the Earth. `precession` rotates their J2000
/// positions to the date.
fn update_planet_positions(
    planets: &mut [Planet],
    julian_date: f64,
    sidereal_time: f64,
    precession: &PrecessionMatrix,
    observer: &Observer,
) {
    let earth = compute_planet_heliocentric(&EARTH_ORBIT, julian_date);
    for planet in planets {
        let geocentric = match planet.kind.orbit() {
            Some(orbit) => compute_planet_heliocentric(orbit, julian_date) - earth,
            None => -earth, // the Sun is (roughly) the origin of the heliocentric frame
        };
        let equatorial = rectangular_to_equatorial(precession.apply(geocentric));
        planet.position = equatorial_to_horizontal(equatorial, sidereal_time, observer);
    }
}

/// Move the Moon to its apparent position, as seen from the Earth's surface, and update its phase. Its elements are
/// already referred to the equinox of date, so it needs no precession.
fn update_moon(moon: &mut Moon, julian_date: f64, sidereal_time: f64, observer: &Observer) {
    // position, corrected for the observer being on the surface rather than at the center of the Earth
    let geocentric = compute_moon_geocentric(moon.orbit, julian_date);
    let position = equatorial_to_horizontal(rectangular_to_equatorial(geocentric), sidereal_time, observer);
    moon.position = correct_for_parallax(position, geocentric.length());

    // phase, from the elongation from the Sun
    let sun = -compute_planet_heliocentric(&EARTH_ORBIT, julian_date);
    moon.phase = moon_age_to_phase(compute_moon_age(geocentric, sun));
}

/// Lift every object by atmospheric refraction, as seen through the air. Run after the positions are updated.
pub fn refract_sky_positions(sky: &mut Sky) {
    for star in &mut sky.stars {
        star.position = apply_refraction(star.position);
    }
    for planet in &mut sky.planets {
        planet.position = apply_refraction(planet.position);
    }
    sky.moon.position = apply_refraction(sky.moon.position);
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::astro::MoonPhase;
    use crate::catalog::load_embedded_catalog;
    use crate::sky::{PlanetKind, Sky};

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
        let mut sky = Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
        update_sky_positions(&mut sky, julian_date, &boston);
        sky
    }

    fn assert_position(actual: crate::astro::Horizontal, azimuth: f64, altitude: f64, epsilon: f64) {
        eprintln!(
            "DEV az {:.5} alt {:.5}",
            actual.azimuth - azimuth,
            actual.altitude - altitude
        );
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
        assert_eq!((vega.catalog_number, vega.name), (7001, Some("Vega")));
        assert_position(vega.position, 0.547246, 0.0, STAR_EPSILON);

        let arcturus = &sky.stars[5339];
        assert_eq!(arcturus.name, Some("Arcturus"));
        assert_position(arcturus.position, 1.511414, 0.440355, STAR_EPSILON);
    }

    #[test]
    fn planet_positions_match_reference() {
        let sky = update_boston_sky();
        let find = |kind| sky.planets.iter().find(|planet| planet.kind == kind).unwrap();
        assert_position(find(PlanetKind::Sun).position, 1.993463, 0.145643, STAR_EPSILON);
        assert_position(find(PlanetKind::Mars).position, 5.1954878, -0.341956, PLANET_EPSILON);
        assert_position(find(PlanetKind::Neptune).position, 5.5390816, -0.779650, PLANET_EPSILON);
    }

    #[test]
    fn refraction_lifts_objects_only() {
        let mut refracted = update_boston_sky();
        refract_sky_positions(&mut refracted);
        let geometric = update_boston_sky();
        let arcturus = (geometric.stars[5339].position, refracted.stars[5339].position);
        assert!(arcturus.1.altitude > arcturus.0.altitude && arcturus.1.azimuth == arcturus.0.azimuth);
        assert!(refracted.planets[0].position.altitude > geometric.planets[0].position.altitude);
        assert!(refracted.moon.position.altitude > geometric.moon.position.altitude);
    }

    #[test]
    fn moon_position_matches_reference() {
        let sky = update_boston_sky();
        assert_position(sky.moon.position, 0.7817126, -1.118899, MOON_EPSILON);
        let first_quarter_soon = [MoonPhase::WaxingCrescent, MoonPhase::FirstQuarter]; // first quarter was at 13:23
        assert!(first_quarter_soon.contains(&sky.moon.phase), "{:?}", sky.moon.phase);
    }
}
