//! Machine-readable current-model output for scripts/reference/generate.py; not an independent reference. Data
//! and stars go through the frame loop's steps 1 and 2 and the stellar motion model, airless.

use astroterm::astro::{
    FrameTime, Horizontal, Observer, Vector3, compute_moon_age, compute_moon_geocentric, compute_planet_heliocentric,
    compute_precession_matrix, models::{BodyId, stars::years_since_j2000},
};
use astroterm::catalog::{EARTH_ORBIT, MOON_ORBIT, load_embedded_catalog};
use astroterm::model::{Data, View};
use astroterm::observation::observe;
use astroterm::solar_system::simulate_solar_system;

fn main() {
    let observer = Observer {
        latitude: 42.3601_f64.to_radians(),
        longitude: -71.0589_f64.to_radians(),
    };
    let catalog = astroterm::sky::prepare_owned_catalog(load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap().catalog;
    let bodies = Data::default();
    let print = |date: f64, name: &str, direction: Vector3| {
        let position = Horizontal::from_vector(direction);
        println!("position,{date:.12},{name},{:.16},{:.16}", position.azimuth, position.altitude);
    };
    for date in [2451545.0, 2459146.0, 2460736.9583333335] {
        let time = FrameTime::from_utc(date);
        let system = simulate_solar_system(&bodies, time, &mut astroterm::time::StepTimes::default());
        let observation = observe(&observer, false, &View::default(), time, &mut astroterm::time::StepTimes::default());
        for name in ["Vega", "Arcturus"] {
            let star = catalog.stars.iter().find(|star| catalog.star_name(star) == Some(name)).unwrap();
            let sample = star.motion.evaluate(years_since_j2000(time.tt), star.magnitude);
            print(date, name, observation.apparent_star_direction(sample.direction));
        }
        for (planet, &position) in bodies.solar_system_planets.planets.iter().zip(&system.planets) {
            if planet.id == BodyId::Earth { continue; }
            print(date, planet.name, observation.apparent_direction(position));
        }
        print(date, "Moon", observation.apparent_direction(system.moon_position(0, observation.observer.position)));
        let moon = compute_moon_geocentric(&MOON_ORBIT, date);
        let sun = -compute_planet_heliocentric(&EARTH_ORBIT, date);
        let old_age = compute_moon_age(moon, sun);
        let same_frame_age = compute_moon_age(moon, compute_precession_matrix(date).apply(sun));
        println!("phase,{date:.12},{old_age:.16},{same_frame_age:.16}");
    }
    for year in [5026.0, 7026.0, 9026.0, 12026.0, 15026.0] {
        let matrix = compute_precession_matrix(2451545.0 + (year - 2000.0) * 365.25);
        for vector in [
            Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            Vector3 { x: 0.0, y: 1.0, z: 0.0 },
            Vector3 { x: 0.0, y: 0.0, z: 1.0 },
        ] {
            let column = matrix.apply(vector);
            println!("precession,{year},{:.16},{:.16},{:.16}", column.x, column.y, column.z);
        }
    }
}
