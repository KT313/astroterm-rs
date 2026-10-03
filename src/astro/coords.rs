//! Conversions between equatorial, horizontal and spherical coordinates.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use super::{Equatorial, Horizontal, Observer, Vector3};

/// Convert equatorial coordinates to the horizontal coordinates of an observer, given Greenwich mean sidereal time.
///
/// Jean Meeus, Astronomical Algorithms, eq. 13.5 & 13.6, modified so West longitudes are negative and azimuth 0 is
/// North (as done by Greg Miller, <https://astrogreg.com/convert_ra_dec_to_alt_az.html>).
pub fn equatorial_to_horizontal(position: Equatorial, sidereal_time: f64, observer: &Observer) -> Horizontal {
    let Equatorial {
        right_ascension,
        declination,
    } = position;
    let latitude = observer.latitude;

    // approximate hour angle (not corrected for nutation), wrapped into (-π, π]
    let local_sidereal_time = (sidereal_time + observer.longitude) % TAU;
    let mut hour_angle = local_sidereal_time - right_ascension;
    if hour_angle < 0.0 {
        hour_angle += TAU;
    }
    if hour_angle > PI {
        hour_angle -= TAU;
    }

    // altitude and azimuth (measured from South), then rotate azimuth 0 to North
    let altitude = (latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour_angle.cos()).asin();
    let azimuth_from_south = hour_angle
        .sin()
        .atan2(hour_angle.cos() * latitude.sin() - declination.tan() * latitude.cos());
    let mut azimuth = azimuth_from_south - PI;
    if azimuth < 0.0 {
        azimuth += TAU;
    }
    Horizontal { azimuth, altitude }
}

/// Convert rectangular equatorial coordinates to right ascension and declination.
pub fn rectangular_to_equatorial(position: Vector3) -> Equatorial {
    let right_ascension = position.y.atan2(position.x);
    let declination = position
        .z
        .atan2((position.x * position.x + position.y * position.y).sqrt());
    Equatorial {
        right_ascension,
        declination,
    }
}

/// Convert horizontal coordinates to spherical (θ measured North of East, Φ from the zenith).
pub fn horizontal_to_spherical(position: Horizontal) -> (f64, f64) {
    (FRAC_PI_2 - position.azimuth, FRAC_PI_2 - position.altitude)
}

/// Correct a geocentric position for parallax: seen from the Earth's surface, a nearby body appears lower in the sky.
/// `distance` is in Earth radii (Schlyter, section 13; altitude only, ignoring the Earth's flattening).
#[cfg(test)]
fn correct_for_parallax(position: Horizontal, distance: f64) -> Horizontal {
    let parallax = (1.0 / distance).asin();
    Horizontal {
        altitude: position.altitude - parallax * position.altitude.cos(),
        ..position
    }
}

/// Correct a geometric position for atmospheric refraction, which lifts objects near the horizon by about 0.5°.
///
/// Saemundsson's formula (Meeus, Astronomical Algorithms, eq. 16.4) for standard pressure and temperature. It diverges
/// below about -5°, so positions under -1° get the lift at -1°, which keeps the mapping continuous and monotonic.
pub fn apply_refraction(position: Horizontal) -> Horizontal {
    let altitude_degrees = position.altitude.to_degrees().max(-1.0);
    let refraction_arcmin = 1.02 / (altitude_degrees + 10.3 / (altitude_degrees + 5.11)).to_radians().tan();
    let refraction = (refraction_arcmin / 60.0).to_radians().max(0.0); // slightly negative near the zenith
    Horizontal {
        altitude: position.altitude + refraction,
        ..position
    }
}

/// The point `angle` radians from `from` along the great circle towards `to`. If the two points coincide or are
/// opposite, there is no unique direction and `from` is returned.
pub fn offset_towards(from: Horizontal, to: Horizontal, angle: f64) -> Horizontal {
    // the tangent at `from` pointing towards `to`
    let (a, b) = (from.to_unit_vector(), to.to_unit_vector());
    let tangent = b - a * a.dot(b);
    let tangent_length = tangent.length();
    if tangent_length < 1e-12 {
        return from;
    }

    // rotate along the great circle
    let (sin, cos) = angle.sin_cos();
    Horizontal::from_vector(a * cos + tangent * (sin / tangent_length))
}

/// Apply refraction without recovering azimuth. The horizontal direction is preserved, including across azimuth 0.
pub fn refract_direction(direction: Vector3) -> Vector3 {
    let horizontal = direction.x.hypot(direction.y);
    let altitude = direction.z.atan2(horizontal);
    let corrected = apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude;
    let (s, c) = corrected.sin_cos();
    if horizontal == 0.0 {
        Vector3 { x: 0.0, y: c, z: s }
    } else {
        Vector3 {
            x: direction.x * (c / horizontal),
            y: direction.y * (c / horizontal),
            z: s,
        }
    }
}

/// Great-circle offset on unit vectors, retaining the legacy degenerate-arc convention.
pub fn offset_vector_towards(from: Vector3, to: Vector3, angle: f64) -> Vector3 {
    let tangent = to - from * from.dot(to);
    let length = tangent.length();
    if length < 1e-12 {
        return from;
    }
    let (s, c) = angle.sin_cos();
    (from * c + tangent * (s / length)).normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TO_RAD: f64 = PI / 180.0;

    #[test]
    fn south_celestial_pole_sits_due_south_at_latitude_altitude() {
        let sydney = Observer {
            latitude: -33.87 * TO_RAD,
            longitude: 0.0,
        };
        let pole = Equatorial {
            right_ascension: 0.0,
            declination: -89.99 * TO_RAD,
        };
        for sidereal_time in [0.0, 1.0, 2.5, 4.0, 5.5] {
            let position = equatorial_to_horizontal(pole, sidereal_time, &sydney);
            assert!((position.altitude - 33.87 * TO_RAD).abs() < 0.02 * TO_RAD);
            assert!((position.azimuth - PI).abs() < 0.1 * TO_RAD);
        }
    }

    #[test]
    fn hour_angle_sign_selects_east_or_west() {
        let sydney = Observer {
            latitude: -33.87 * TO_RAD,
            longitude: 0.0,
        };
        let (sidereal_time, hour_angle) = (1.0, 30.0 * TO_RAD);
        let east = Equatorial {
            right_ascension: sidereal_time + hour_angle,
            declination: -60.0 * TO_RAD,
        };
        let west = Equatorial {
            right_ascension: sidereal_time - hour_angle,
            declination: -60.0 * TO_RAD,
        };
        assert!(equatorial_to_horizontal(east, sidereal_time, &sydney).azimuth < PI);
        assert!(equatorial_to_horizontal(west, sidereal_time, &sydney).azimuth > PI);
    }

    #[test]
    fn parallax_lowers_nearby_objects_most_at_the_horizon() {
        let horizon = correct_for_parallax(
            Horizontal {
                azimuth: 1.0,
                altitude: 0.0,
            },
            60.0,
        );
        assert!((horizon.altitude + (1.0_f64 / 60.0).asin()).abs() < 1e-12);
        assert_eq!(horizon.azimuth, 1.0);
        let zenith = correct_for_parallax(
            Horizontal {
                azimuth: 1.0,
                altitude: FRAC_PI_2,
            },
            60.0,
        );
        assert!((zenith.altitude - FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn refraction_lifts_objects_near_the_horizon_most() {
        let lift_at = |altitude_degrees: f64| {
            let position = Horizontal {
                azimuth: 1.0,
                altitude: altitude_degrees * TO_RAD,
            };
            let refracted = apply_refraction(position);
            assert_eq!(refracted.azimuth, 1.0);
            (refracted.altitude - position.altitude) / TO_RAD * 60.0 // in arcminutes
        };
        assert!((lift_at(0.0) - 28.98).abs() < 0.05, "{}", lift_at(0.0));
        assert!((lift_at(45.0) - 1.0).abs() < 0.05, "{}", lift_at(45.0));
        assert_eq!(lift_at(90.0), 0.0);
        assert!((lift_at(-10.0) - lift_at(-1.0)).abs() < 1e-9);

        // higher true altitudes always appear higher
        let apparent: Vec<f64> = (-100..=900)
            .map(|tenth_degree| {
                apply_refraction(Horizontal {
                    azimuth: 0.0,
                    altitude: f64::from(tenth_degree) / 10.0 * TO_RAD,
                })
            })
            .map(|position| position.altitude)
            .collect();
        assert!(apparent.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn offset_towards_moves_along_the_great_circle() {
        let east = Horizontal {
            azimuth: FRAC_PI_2,
            altitude: 0.0,
        };
        let zenith = Horizontal {
            azimuth: 0.0,
            altitude: FRAC_PI_2,
        };
        let step = offset_towards(east, zenith, 10.0 * TO_RAD);
        assert!((step.azimuth - FRAC_PI_2).abs() < 1e-9 && (step.altitude - 10.0 * TO_RAD).abs() < 1e-9);

        let north = Horizontal {
            azimuth: 0.0,
            altitude: 0.0,
        };
        let step = offset_towards(north, east, 30.0 * TO_RAD);
        assert!((step.azimuth - 30.0 * TO_RAD).abs() < 1e-9 && step.altitude.abs() < 1e-9);
        assert_eq!(offset_towards(north, north, 0.1), north);
    }

    #[test]
    fn rectangular_to_equatorial_recovers_angles() {
        let position = rectangular_to_equatorial(Vector3 { x: 0.0, y: 1.0, z: 1.0 });
        assert!((position.right_ascension - FRAC_PI_2).abs() < 1e-12);
        assert!((position.declination - PI / 4.0).abs() < 1e-12);
    }
}
