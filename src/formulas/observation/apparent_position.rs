//! Conversions between equatorial, horizontal and spherical coordinates.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::formulas::constants::UNREFRACTION_STEPS;
use crate::formulas::{BodyState, Equatorial, Horizontal, Matrix3, Observer, Unrefraction, Vector3};

/// Apparent horizon direction of a barycentric body position, including observer displacement.
pub fn compute_apparent_body_direction(position: Vector3, observer: BodyState, to_horizon: Matrix3, atmosphere: bool) -> Vector3 {
    compute_apparent_star_direction((position - observer.position).normalized(), observer.velocity, to_horizon, atmosphere)
}

/// Aberration, orientation and optional refraction of a unit catalog direction, in that order.
pub fn compute_apparent_star_direction(direction: Vector3, observer_velocity: Vector3, to_horizon: Matrix3, atmosphere: bool) -> Vector3 {
    let horizon = to_horizon.apply(apply_aberration(direction, observer_velocity));
    if atmosphere { refract_direction(horizon) } else { horizon }
}

/// Undo refraction, orientation and aberration to recover a unit catalog direction.
pub fn compute_catalog_star_direction(direction: Vector3, observer_velocity: Vector3, to_catalog: Matrix3, atmosphere: bool) -> Vector3 {
    let horizon = if atmosphere { unrefract_direction(direction) } else { direction };
    remove_aberration(to_catalog.apply(horizon), observer_velocity)
}

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

/// First-order annual plus diurnal aberration of a direction seen by an observer moving at `velocity` (AU/day);
/// the error is O(β²), below 0.002 arcseconds for the Earth.
pub fn apply_aberration(direction: Vector3, velocity: Vector3) -> Vector3 {
    (direction.normalized() + velocity * (1.0 / crate::formulas::LIGHT_SPEED_AU_DAY)).normalized()
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

/// Undo `refract_direction` (see `Unrefraction::unrefract`). Callers with many directions resolve the table once.
pub fn unrefract_direction(direction: Vector3) -> Vector3 { Unrefraction::resolve().unrefract(direction) }

impl Unrefraction {
    /// Resolve the shared table, building it on first use.
    pub fn resolve() -> Self { Self { table: &UNREFRACTION } }

    /// Undo `refract_direction`: the geometric direction whose refracted image is `direction`. The inverse of
    /// Saemundsson's formula has no closed form, so it is read from a table over sin(apparent altitude), 8193
    /// entries built once from the forward formula. Linear interpolation keeps it within 0.01″ of the direct solve
    /// from the horizon up to 88.5°, within 0.5″ in the last degrees to the zenith (where the table's altitude steps
    /// widen and the lift itself is under 1″) and within 2″ around the formula's −1° clamp. This is a constant
    /// table of the formula, not a frame cache: it depends on nothing that changes at runtime.
    #[inline]
    pub fn unrefract(self, direction: Vector3) -> Vector3 {
        // Look up sin(true altitude) and the horizontal scale cos(true) / cos(apparent) for this apparent altitude.
        let z = direction.z.clamp(-1.0, 1.0);
        let position = (z + 1.0) * (UNREFRACTION_STEPS as f64 / 2.0);
        let index = (position as usize).min(UNREFRACTION_STEPS - 1);
        let fraction = position - index as f64;
        let [[sine_low, scale_low], [sine_high, scale_high]] = [self.table[index], self.table[index + 1]];
        let sine = sine_low + (sine_high - sine_low) * fraction;
        let scale = scale_low + (scale_high - scale_low) * fraction;
        Vector3 { x: direction.x * scale, y: direction.y * scale, z: sine }
    }
}

/// Per table entry at sin(apparent altitude) = −1 + 2i / steps: sin(true altitude), cos(true) / cos(apparent).
static UNREFRACTION: std::sync::LazyLock<[[f64; 2]; UNREFRACTION_STEPS + 1]> = std::sync::LazyLock::new(build_unrefraction_table);

/// Build the fixed inverse-refraction samples once, in ascending apparent altitude.
fn build_unrefraction_table() -> [[f64; 2]; UNREFRACTION_STEPS + 1] {
    let mut table = [[0.0; 2]; UNREFRACTION_STEPS + 1];
    for (index, entry) in table.iter_mut().enumerate() {
        let apparent_sine = (-1.0 + 2.0 * index as f64 / UNREFRACTION_STEPS as f64).clamp(-1.0, 1.0);
        let apparent = apparent_sine.asin();
        let altitude = unrefract_altitude(apparent).max(-FRAC_PI_2);
        let (sine, cosine) = altitude.sin_cos();
        let apparent_cosine = apparent.cos();
        let scale = if apparent_cosine < 1e-9 { 1.0 } else { cosine / apparent_cosine }; // at the poles the horizontal part is zero anyway
        *entry = [sine, scale];
    }
    table
}

/// The geometric altitude that `apply_refraction` lifts to `apparent`: Newton's method on the monotone formula.
fn unrefract_altitude(apparent: f64) -> f64 {
    let mut altitude = apparent - compute_refraction_residual(apparent, apparent);                                           // start one lift below the apparent altitude
    for _ in 0..8 {
        let step = compute_refraction_residual(altitude, apparent) / refraction_derivatives(altitude).0;
        altitude -= step;
        if step.abs() < 1e-14 { break; }
    }
    altitude
}

/// Difference between a refracted trial altitude and the requested apparent altitude.
fn compute_refraction_residual(altitude: f64, apparent: f64) -> f64 {
    apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - apparent
}

/// Undo `apply_aberration` to the same first order: the catalog direction of an aberrated one.
pub fn remove_aberration(direction: Vector3, velocity: Vector3) -> Vector3 {
    (direction.normalized() - velocity * (1.0 / crate::formulas::LIGHT_SPEED_AU_DAY)).normalized()
}

/// First and second derivatives of corrected altitude with respect to geometric altitude (radians).
/// The clamped lift is constant below -1 degree and after its positive part reaches zero.
fn refraction_derivatives(altitude: f64) -> (f64, f64) {
    // Use the same clamp and positive-lift branches as apply_refraction.
    let degrees = altitude.to_degrees();
    let clamped_degrees = degrees.max(-1.0);
    let argument = (clamped_degrees + 10.3 / (clamped_degrees + 5.11)).to_radians();
    if degrees <= -1.0 || 1.02 / argument.tan() <= 0.0 { return (1.0, 0.0); }

    // Differentiate the cotangent lift, taking care that the formula's coefficients use degrees.
    let coefficient = (1.02_f64 / 60.0).to_radians();
    let argument_slope = 1.0 - 10.3 / (clamped_degrees + 5.11).powi(2);
    let argument_curvature = (20.6 / (clamped_degrees + 5.11).powi(3)).to_degrees();
    let cosecant_squared = 1.0 / argument.sin().powi(2);
    let slope = 1.0 - coefficient * argument_slope / argument.sin().powi(2);
    let curvature = coefficient * cosecant_squared * (2.0 * argument_slope.powi(2) / argument.tan() - argument_curvature);
    (slope, curvature)
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
    use crate::formulas::constants::DEGREES_TO_RADIANS as TO_RAD;

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

    fn compute_refraction_lift_arcminutes(altitude_degrees: f64) -> f64 {
        let position = Horizontal {
            azimuth: 1.0,
            altitude: altitude_degrees * TO_RAD,
        };
        let refracted = apply_refraction(position);
        assert_eq!(refracted.azimuth, 1.0);
        (refracted.altitude - position.altitude) / TO_RAD * 60.0 // in arcminutes
    }

    #[test]
    fn refraction_lifts_objects_near_the_horizon_most() {
        assert!((compute_refraction_lift_arcminutes(0.0) - 28.98).abs() < 0.05, "{}", compute_refraction_lift_arcminutes(0.0));
        assert!((compute_refraction_lift_arcminutes(45.0) - 1.0).abs() < 0.05, "{}", compute_refraction_lift_arcminutes(45.0));
        assert_eq!(compute_refraction_lift_arcminutes(90.0), 0.0);
        assert!((compute_refraction_lift_arcminutes(-10.0) - compute_refraction_lift_arcminutes(-1.0)).abs() < 1e-9);

        // higher true altitudes always appear higher
        let mut apparent = Vec::with_capacity(1001);
        for tenth_degree in -100..=900 {
            let refracted = apply_refraction(Horizontal {
                azimuth: 0.0,
                altitude: f64::from(tenth_degree) / 10.0 * TO_RAD,
            });
            apparent.push(refracted.altitude);
        }
        for pair in apparent.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn vector_offset_moves_along_the_great_circle() {
        let east = Horizontal {
            azimuth: FRAC_PI_2,
            altitude: 0.0,
        };
        let zenith = Horizontal {
            azimuth: 0.0,
            altitude: FRAC_PI_2,
        };
        let step = Horizontal::from_vector(offset_vector_towards(east.to_unit_vector(), zenith.to_unit_vector(), 10.0 * TO_RAD));
        assert!((step.azimuth - FRAC_PI_2).abs() < 1e-9 && (step.altitude - 10.0 * TO_RAD).abs() < 1e-9);

        let north = Horizontal {
            azimuth: 0.0,
            altitude: 0.0,
        };
        let step = Horizontal::from_vector(offset_vector_towards(north.to_unit_vector(), east.to_unit_vector(), 30.0 * TO_RAD));
        assert!((step.azimuth - 30.0 * TO_RAD).abs() < 1e-9 && step.altitude.abs() < 1e-9);
        assert_eq!(Horizontal::from_vector(offset_vector_towards(north.to_unit_vector(), north.to_unit_vector(), 0.1)), north);
    }

    #[test]
    fn rectangular_to_equatorial_recovers_angles() {
        let position = rectangular_to_equatorial(Vector3 { x: 0.0, y: 1.0, z: 1.0 });
        assert!((position.right_ascension - FRAC_PI_2).abs() < 1e-12);
        assert!((position.declination - PI / 4.0).abs() < 1e-12);
    }

    #[test]
    fn unrefraction_inverts_refraction_at_every_altitude_including_the_clamp_and_the_poles() {
        // Round trips through the table stay within 0.01″ down to 85° below the horizon; within the last few
        // degrees above the nadir the constant clamped lift makes the inverse ill-conditioned, and it only has to
        // stay finite and bounded there.
        let (mut worst_above, mut worst_below): (f64, f64) = (0.0, 0.0);
        for tenth_degree in -899..=900 {
            let altitude = tenth_degree as f64 / 10.0 * TO_RAD;
            for azimuth in [0.0, 1.0, 2.5, 4.0] {
                let geometric = Horizontal { azimuth, altitude }.to_unit_vector();
                let apparent = refract_direction(geometric);
                let back = unrefract_direction(apparent);
                assert!(back.length().is_finite() && back.length() < 1.01, "{altitude} {azimuth}: {back:?}");
                if tenth_degree < -850 { continue; }
                let error = (back.normalized() - geometric).length();
                if (0..=885).contains(&tenth_degree) { worst_above = worst_above.max(error); } else { worst_below = worst_below.max(error); }
                assert!((back.length() - 1.0).abs() < 1e-6, "{altitude} {azimuth}: {back:?}");       // interpolation leaves the length slightly off
            }
        }
        assert!(worst_above < 0.01 / 3600.0 * TO_RAD, "{} arcseconds between the horizon and 88.5°", worst_above / TO_RAD * 3600.0);
        assert!(worst_below < 3.0 / 3600.0 * TO_RAD, "{} arcseconds elsewhere", worst_below / TO_RAD * 3600.0); // the clamp's kink at −1° and the coarse altitude steps near the zenith
        let zenith = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        assert!((unrefract_direction(zenith) - zenith).length() < 1e-9);
        let nadir = Vector3 { x: 0.0, y: 0.0, z: -1.0 };
        assert!(unrefract_direction(nadir).z <= -0.9999 && unrefract_direction(nadir).length() <= 1.0 + 1e-9);
        let lifted = unrefract_direction(Horizontal { azimuth: 0.3, altitude: 0.0 }.to_unit_vector());
        assert!(lifted.z < -(28.0 / 60.0 * TO_RAD).sin() && lifted.z > -(36.0 / 60.0 * TO_RAD).sin(), "{lifted:?}"); // about 34′ below the apparent horizon
    }

    #[test]
    fn resolved_unrefraction_matches_the_free_function_bits() {
        // Every height from below the nadir to above the zenith (clamped), and both ends of the table exactly.
        let unrefraction = Unrefraction::resolve();
        let heights = (0..=100_000).map(|step| -1.2 + 2.4 * step as f64 / 100_000.0).chain([-1.0, 1.0, 1.0_f64.next_down(), (-1.0_f64).next_up()]);
        for z in heights {
            let direction = Vector3 { x: 0.6 * (1.0 - z * z).abs().sqrt(), y: -0.8 * (1.0 - z * z).abs().sqrt(), z };
            let (resolved, free) = (unrefraction.unrefract(direction), unrefract_direction(direction));
            assert_eq!([resolved.x, resolved.y, resolved.z].map(f64::to_bits), [free.x, free.y, free.z].map(f64::to_bits), "z = {z}");
        }
    }

    #[test]
    fn removing_aberration_inverts_applying_it_to_first_order() {
        let velocity = Vector3 { x: 0.0172, y: 0.001, z: -0.0004 };                            // Earth's orbital speed in AU/day
        for direction in [Vector3 { x: 1.0, y: 0.0, z: 0.0 }, Vector3 { x: 0.3, y: -0.5, z: 0.8 }, Vector3 { x: -0.2, y: 0.9, z: -0.4 }] {
            let direction = direction.normalized();
            let back = remove_aberration(apply_aberration(direction, velocity), velocity);
            assert!((back - direction).length() < 0.01 / 3600.0 * TO_RAD, "{direction:?}: {back:?}");
        }
    }
}
