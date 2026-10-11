//! Azimuthal map projections onto the unit disk.
//!
//! Reference: John P. Snyder, Map Projections - A Working Manual (<https://pubs.usgs.gov/pp/1395/report.pdf>).

use std::f64::consts::{FRAC_PI_2, PI};

use crate::astro::Horizontal;

# [doc = " A point on the projection plane: radius and angle measured counterclockwise from the positive x-axis (right)."] # [doc = " Radius 1 is the edge of the rendered circle."] use crate::model::Polar;

/// Stereographic projection centered on the North pole of a sphere with radius `sphere_radius`, given spherical
/// coordinates (θ North of East, Φ from the pole). The equator lands on the circle with radius `sphere_radius`.
///
/// The angle is mirrored and rotated so that, for horizontal coordinates, North is at the top of the projection.
pub fn project_stereographic_north(sphere_radius: f64, theta: f64, phi: f64) -> Polar {
    let angular_distance = phi.abs(); // from the pole (Φ = 0)
    Polar {
        radius: sphere_radius * (angular_distance / 2.0).tan(),
        theta: PI - theta,
    } // Snyder eq. (21-1), (20-2)
}

/// Stereographic projection centered on `center`, as seen looking outward, so azimuth increases to the right. Points
/// within π/2 of the center land inside the unit circle.
pub fn project_stereographic_horizontal(position: Horizontal, center: Horizontal) -> Polar {
    let (cos_c, x, y) = compute_azimuthal_terms(position, center);
    Polar {
        radius: (cos_c.acos() / 2.0).tan(),
        theta: y.atan2(x),
    }
}

/// Azimuthal equidistant projection with the same orientation and scale as [`project_stereographic_horizontal`]
/// (π/2 from the center lands on r = 1), but the radius grows linearly with the angle, so the point directly behind
/// the center lands on the r = 2 circle instead of at infinity.
///
/// The point directly behind has no direction; it is placed at θ = π/2 (the top of the r = 2 circle).
pub fn project_equidistant_horizontal(position: Horizontal, center: Horizontal) -> Polar {
    let (cos_c, x, y) = compute_azimuthal_terms(position, center);
    let radius = cos_c.acos() / FRAC_PI_2; // ρ = c (Snyder ch. 25), scaled so 90° -> 1
    let directly_behind = cos_c < 0.0 && x.hypot(y) < 1e-12;
    let theta = if directly_behind { FRAC_PI_2 } else { y.atan2(x) };
    Polar { radius, theta }
}

/// cos of the angular distance from `center` to `position` (Snyder eq. 5-3), and the direction terms of eq. 21-2/3.
/// The direction vector (x, y) has length sin(c).
fn compute_azimuthal_terms(position: Horizontal, center: Horizontal) -> (f64, f64, f64) {
    let delta_azimuth = position.azimuth - center.azimuth;
    let (sin_alt, cos_alt) = (position.altitude.sin(), position.altitude.cos());
    let (sin_center, cos_center) = (center.altitude.sin(), center.altitude.cos());

    let cos_c = (sin_center * sin_alt + cos_center * cos_alt * delta_azimuth.cos()).clamp(-1.0, 1.0);
    let x = cos_alt * delta_azimuth.sin();
    let y = cos_center * sin_alt - sin_center * cos_alt * delta_azimuth.cos();
    (cos_c, x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{Equatorial, Observer, equatorial_to_horizontal, horizontal_to_spherical};

    const TO_RAD: f64 = PI / 180.0;

    fn horizontal(azimuth: f64, altitude: f64) -> Horizontal {
        Horizontal { azimuth, altitude }
    }

    fn angle_difference(a: f64, b: f64) -> f64 {
        let d = (a - b) % (2.0 * PI);
        if d > PI {
            d - 2.0 * PI
        } else if d < -PI {
            d + 2.0 * PI
        } else {
            d
        }
    }

    fn project_overhead(position: Horizontal) -> Polar {
        let (theta, phi) = horizontal_to_spherical(position);
        project_stereographic_north(1.0, theta, phi)
    }

    #[test]
    fn stereographic_horizontal_places_cardinal_points_on_edges() {
        let center = horizontal(334.0 * TO_RAD, 0.0);
        let project = |azimuth, altitude| project_stereographic_horizontal(horizontal(azimuth, altitude), center);

        assert!(project(center.azimuth, 0.0).radius.abs() < 0.01);

        let right = project(center.azimuth + FRAC_PI_2, 0.0);
        assert!((right.radius - 1.0).abs() < 0.01 && right.theta.abs() < 0.01);

        let left = project(center.azimuth - FRAC_PI_2, 0.0);
        assert!((left.radius - 1.0).abs() < 0.01 && (left.theta.abs() - PI).abs() < 0.01);

        let zenith = project(0.0, FRAC_PI_2);
        assert!((zenith.radius - 1.0).abs() < 0.01 && (zenith.theta - FRAC_PI_2).abs() < 0.01);

        let nadir = project(0.0, -FRAC_PI_2);
        assert!((nadir.radius - 1.0).abs() < 0.01 && (nadir.theta + FRAC_PI_2).abs() < 0.01);

        assert!(project(center.azimuth + PI, 0.0).radius > 1.0);
        assert!(project(center.azimuth + PI, 0.1).radius > 1.0);
    }

    #[test]
    fn stereographic_horizontal_matches_reference_points() {
        // Polaris from Tokyo (alt ~35.7°, az 0°): up and right when facing 334°
        let polaris = project_stereographic_horizontal(horizontal(0.0, 35.7 * TO_RAD), horizontal(334.0 * TO_RAD, 0.0));
        let (x, y) = polaris.to_cartesian();
        assert!((x - 0.206).abs() < 0.01 && (y - 0.337).abs() < 0.01);

        // wraps around North: facing 350°, az 10° is to the right
        let wrapped = project_stereographic_horizontal(horizontal(10.0 * TO_RAD, 0.0), horizontal(350.0 * TO_RAD, 0.0));
        assert!((wrapped.radius - (10.0 * TO_RAD).tan()).abs() < 0.01 && wrapped.theta.abs() < 0.01);
    }

    #[test]
    fn stereographic_horizontal_tilted_90_equals_overhead_view() {
        let points = [
            (0.0, 30.0),
            (45.0, 10.0),
            (100.0, 60.0),
            (200.0, 5.0),
            (300.0, 80.0),
            (250.0, -20.0),
        ];
        for (azimuth, altitude) in points {
            let position = horizontal(azimuth * TO_RAD, altitude * TO_RAD);
            let overhead = project_overhead(position);

            let south = project_stereographic_horizontal(position, horizontal(PI, FRAC_PI_2));
            assert!((south.radius - overhead.radius).abs() < 1e-6);
            assert!(angle_difference(south.theta, overhead.theta).abs() < 1e-6);

            for north_azimuth in [0.0, 2.0 * PI] {
                let north = project_stereographic_horizontal(position, horizontal(north_azimuth, FRAC_PI_2));
                assert!((north.radius - overhead.radius).abs() < 1e-6);
                assert!((angle_difference(north.theta, overhead.theta).abs() - PI).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn stereographic_horizontal_with_tilt_puts_horizon_below_center() {
        let (azimuth, tilt) = (334.0 * TO_RAD, 10.0 * TO_RAD);
        let center = horizontal(azimuth, tilt);
        assert!(project_stereographic_horizontal(center, center).radius.abs() < 1e-6);

        let horizon = project_stereographic_horizontal(horizontal(azimuth, 0.0), center);
        assert!((horizon.radius - (tilt / 2.0).tan()).abs() < 1e-6);
        assert!((horizon.theta + FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn stereographic_horizontal_southern_hemisphere_orientation() {
        let sydney = Observer {
            latitude: -33.87 * TO_RAD,
            longitude: 0.0,
        };
        let (facing_south, facing_north) = (PI, 0.0);
        let pole = Equatorial {
            right_ascension: 0.0,
            declination: -89.99 * TO_RAD,
        };

        // the south celestial pole is straight up from the center when facing S
        for sidereal_time in [0.0, 1.0, 2.5, 4.0, 5.5] {
            let position = equatorial_to_horizontal(pole, sidereal_time, &sydney);
            let projected = project_stereographic_horizontal(position, horizontal(facing_south, 0.0));
            let (x, y) = projected.to_cartesian();
            assert!(x.abs() < 0.005 && y > 0.0);
            assert!((projected.radius - (33.87 * TO_RAD / 2.0).tan()).abs() < 0.005);

            let tilted = project_stereographic_horizontal(position, horizontal(facing_south, 33.87 * TO_RAD));
            assert!(tilted.radius.abs() < 0.005);
        }

        // facing S: east of the meridian is left, west is right, at the same height
        let (sidereal_time, hour_angle) = (1.0, 30.0 * TO_RAD);
        let project_at = |right_ascension: f64, declination: f64, facing: f64| {
            let star = Equatorial {
                right_ascension,
                declination,
            };
            let position = equatorial_to_horizontal(star, sidereal_time, &sydney);
            project_stereographic_horizontal(position, horizontal(facing, 0.0)).to_cartesian()
        };
        let (x_east, y_east) = project_at(sidereal_time + hour_angle, -60.0 * TO_RAD, facing_south);
        let (x_west, y_west) = project_at(sidereal_time - hour_angle, -60.0 * TO_RAD, facing_south);
        assert!(x_east < 0.0 && x_west > 0.0);
        assert!((x_east + x_west).abs() < 1e-6 && (y_east - y_west).abs() < 1e-6);

        // facing N: east is right, west is left
        let (x_east, y_east) = project_at(sidereal_time + hour_angle, 0.0, facing_north);
        assert!(x_east > 0.0 && y_east > 0.0);
        let (x_west, _) = project_at(sidereal_time - hour_angle, 0.0, facing_north);
        assert!(x_west < 0.0);
    }

    #[test]
    fn equidistant_horizontal_is_linear_in_angle() {
        let center = horizontal(334.0 * TO_RAD, 0.0);
        assert!(project_equidistant_horizontal(center, center).radius.abs() < 1e-6);

        // 90° away -> unit circle, same direction as stereographic
        for (delta_azimuth, altitude) in [(FRAC_PI_2, 0.0), (-FRAC_PI_2, 0.0), (0.0, FRAC_PI_2), (0.0, -FRAC_PI_2)] {
            let position = horizontal(center.azimuth + delta_azimuth, altitude);
            let equidistant = project_equidistant_horizontal(position, center);
            let stereographic = project_stereographic_horizontal(position, center);
            assert!((equidistant.radius - 1.0).abs() < 1e-6);
            assert!(angle_difference(equidistant.theta, stereographic.theta).abs() < 1e-6);
        }

        let half = project_equidistant_horizontal(horizontal(center.azimuth + 45.0 * TO_RAD, 0.0), center);
        assert!((half.radius - 0.5).abs() < 1e-6);
        let third = project_equidistant_horizontal(horizontal(center.azimuth, 30.0 * TO_RAD), center);
        assert!((third.radius - 1.0 / 3.0).abs() < 1e-6 && (third.theta - FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn equidistant_horizontal_tilted_90_equals_overhead_direction() {
        for (azimuth, altitude) in [(0.0, 30.0), (100.0, 60.0), (200.0, 5.0), (250.0, -20.0)] {
            let position = horizontal(azimuth * TO_RAD, altitude * TO_RAD);
            let projected = project_equidistant_horizontal(position, horizontal(PI, FRAC_PI_2));
            assert!((projected.radius - (90.0 - altitude) / 90.0).abs() < 1e-6);
            assert!(angle_difference(projected.theta, project_overhead(position).theta).abs() < 1e-6);
        }
    }

    #[test]
    fn equidistant_horizontal_point_behind_is_on_outer_circle_at_fixed_angle() {
        let center = horizontal(334.0 * TO_RAD, 0.0);
        let behind = project_equidistant_horizontal(horizontal(center.azimuth + PI, 0.0), center);
        assert!((behind.radius - 2.0).abs() < 1e-6);
        assert_eq!(behind.theta, FRAC_PI_2);

        let almost_behind = project_equidistant_horizontal(horizontal(center.azimuth + PI - 0.01, 0.0), center);
        assert!(almost_behind.radius < 2.0 && almost_behind.theta.abs() < 1e-6);
    }
}
