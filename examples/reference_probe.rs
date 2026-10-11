//! Machine-readable current-model output for scripts/reference/generate.py; not an independent reference.

use astroterm::astro::{
    Observer, Vector3, compute_moon_age, compute_moon_geocentric, compute_planet_heliocentric,
    compute_precession_matrix,
};
use astroterm::catalog::{EARTH_ORBIT, MOON_ORBIT, load_embedded_catalog};
use astroterm::sky::update_sky_positions;
use astroterm::timing::StepTimes;

fn main() {
    let observer = Observer {
        latitude: 42.3601_f64.to_radians(),
        longitude: -71.0589_f64.to_radians(),
    };
    let mut sky = astroterm::sky::create_sky_from_catalog(&load_embedded_catalog().unwrap()).unwrap();
    for date in [2451545.0, 2459146.0, 2460736.9583333335] {
        update_sky_positions(&mut sky, date, &observer, f64::INFINITY, &mut StepTimes::default());
        for name in ["Vega", "Arcturus"] {
            let star = sky.stars.iter().find(|star| sky.star_name(star) == Some(name)).unwrap();
            println!(
                "position,{date:.12},{name},{:.16},{:.16}",
                star.horizontal_position().azimuth,
                star.horizontal_position().altitude
            );
        }
        for planet in &sky.planets {
            println!(
                "position,{date:.12},{},{:.16},{:.16}",
                planet.kind.name(),
                planet.horizontal_position().azimuth,
                planet.horizontal_position().altitude
            );
        }
        println!(
            "position,{date:.12},Moon,{:.16},{:.16}",
            sky.moon.horizontal_position().azimuth,
            sky.moon.horizontal_position().altitude
        );
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
