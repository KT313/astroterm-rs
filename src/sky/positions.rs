//! One-shot direct evaluation retained for reference fixtures and library compatibility. The runtime frame loop
//! owns persistent SimulationState and invokes simulation, observation and projection explicitly.
use super::{FrameTime, SimulationState, Sky, observe_sky, prepare_observer, update_simulation};
use crate::astro::{Observer, apply_refraction};
use crate::timing::StepTimes;

pub fn update_sky_positions(
    sky: &mut Sky,
    julian_date_ut1: f64,
    observer: &Observer,
    magnitude_threshold: f32,
    times: &mut StepTimes,
) {
    let time = FrameTime::from_utc(julian_date_ut1);
    let mut simulation = SimulationState::exact();
    update_simulation(&mut simulation, time, &[], times).expect("finite reference epoch");
    let observer = prepare_observer(&simulation, time, *observer).expect("prepared reference state");
    observe_sky(&simulation, &observer, magnitude_threshold, false, sky, times).expect("prepared reference state");
}

/// Compatibility correction pass, idempotent for a prepared observed sky. Production requests refraction in observe_sky.
pub fn refract_sky_positions(sky: &mut Sky) {
    if sky.refracted {
        return;
    }
    for star in &mut sky.stars {
        star.position = apply_refraction(star.position);
    }
    for planet in &mut sky.planets {
        planet.position = apply_refraction(planet.position);
    }
    sky.moon.position = apply_refraction(sky.moon.position);
    sky.refracted = true;
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::astro::MoonPhase;
    use crate::astro::Observer;
    use crate::catalog::{Designation, load_embedded_catalog};
    use crate::sky::{PlanetKind, Sky};

    #[test]
    fn only_the_inclusive_brightness_prefix_is_updated_and_refracted() {
        let mut sky = Sky::from_catalog(&load_embedded_catalog().unwrap());
        let threshold = 5.0;
        let count = sky.count_bright_stars(threshold);
        let sentinel = crate::astro::Horizontal {
            azimuth: 123.0,
            altitude: -123.0,
        };
        for star in &mut sky.stars {
            star.position = sentinel;
        }
        update_sky_positions(
            &mut sky,
            2451545.0,
            &Observer::default(),
            threshold,
            &mut StepTimes::default(),
        );
        refract_sky_positions(&mut sky);
        assert!(sky.stars[..count].iter().all(|s| s.position != sentinel));
        assert_eq!(sky.stars.len(), count);
        assert_eq!(sky.stars[count - 1].magnitude, threshold);
    }

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
        update_sky_positions(&mut sky, julian_date, &boston, f32::INFINITY, &mut StepTimes::default());
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
        // Historical C fixtures retained. Independent phase-0 ERFA checks put Vega below the horizon (-0.362°
        // apparent, -0.366° mean); the original exact 0.0 altitude has no reproducible correction settings.
        // See tests/position_references.rs and scripts/reference/README.md for the airless audit.
        let sky = update_boston_sky();
        let vega = sky.stars.iter().find(|star| star.id.0 == 7001).unwrap();
        assert_eq!(
            (vega.designation, sky.star_name(vega)),
            (Some(Designation::Hr(7001)), Some("Vega"))
        );
        assert_position(vega.position, 0.547246, 0.0, STAR_EPSILON);

        let arcturus = sky.stars.iter().find(|star| star.id.0 == 5340).unwrap();
        assert_eq!(sky.star_name(arcturus), Some("Arcturus"));
        assert_position(arcturus.position, 1.511414, 0.440355, STAR_EPSILON);
    }

    #[test]
    fn planet_positions_match_reference() {
        // Historical C fixtures retained: their Mars/Neptune altitudes differ from fresh airless Horizons queries
        // by about 0.82°/0.66°. Our differences from those queries are only about 0.06°/0.02°; the larger old
        // deviations are not evidence of similarly large current model errors. Historical refraction is unknown.
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
        let arcturus = (
            geometric.stars.iter().find(|s| s.id.0 == 5340).unwrap().position,
            refracted.stars.iter().find(|s| s.id.0 == 5340).unwrap().position,
        );
        assert!(arcturus.1.altitude > arcturus.0.altitude && arcturus.1.azimuth == arcturus.0.azimuth);
        assert!(refracted.planets[0].position.altitude > geometric.planets[0].position.altitude);
        assert!(refracted.moon.position.altitude > geometric.moon.position.altitude);
    }

    #[test]
    fn moon_position_matches_reference() {
        // Historical C fixture retained. Horizons now gives -65.8892° airless topocentric altitude; our -65.9220°
        // is much closer than the old -64.1082° value. Its original settings cannot be reconstructed reliably.
        let sky = update_boston_sky();
        assert_position(sky.moon.position, 0.7817126, -1.118899, MOON_EPSILON);
        let first_quarter_soon = [MoonPhase::WaxingCrescent, MoonPhase::FirstQuarter]; // first quarter was at 13:23
        assert!(first_quarter_soon.contains(&sky.moon.phase), "{:?}", sky.moon.phase);
    }
}
