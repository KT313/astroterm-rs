//! WGS84 sea-level geodetic site and rotational velocity in Earth-fixed axes.
use crate::astro::models::BodyState;
use crate::astro::{Observer, Vector3};

/// WGS84 a = 6378137 m, 1/f = 298.257223563. No elevation or polar motion model.
/// Velocity is the inertial spin velocity expressed in fixed axes, AU/day (not the fixed-coordinate derivative).
pub fn compute_site_state(site: Observer) -> BodyState {
    let f = 1.0 / 298.257223563;
    let e2 = f * (2.0 - f);
    let (s, c) = site.latitude.sin_cos();
    let (sl, cl) = site.longitude.sin_cos();
    let n = (6378137.0 / 149597870700.0) / (1.0 - e2 * s * s).sqrt();
    let position = Vector3 {
        x: n * c * cl,
        y: n * c * sl,
        z: n * (1.0 - e2) * s,
    };
    let omega = std::f64::consts::TAU * 1.002_737_811_911_354_6;
    BodyState {
        position,
        velocity: Vector3 {
            x: -omega * position.y,
            y: omega * position.x,
            z: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wgs84_equator_pole_and_mid_latitude_match_reference_geometry() {
        let equator = compute_site_state(Observer::default());
        let pole = compute_site_state(Observer {
            latitude: std::f64::consts::FRAC_PI_2,
            longitude: 0.0,
        });
        assert!((equator.position.length() * 149597870700.0 - 6378137.0).abs() < 1e-6);
        assert!((pole.position.length() * 149597870700.0 - 6356752.314245179).abs() < 1e-6);
        assert!(pole.velocity.length() < 1e-18);
        let mid = compute_site_state(Observer {
            latitude: 45_f64.to_radians(),
            longitude: 0.0,
        });
        // ERFA gd2gc(WGS84, 0, pi/4, 0), metres.
        assert!((mid.position.x * 149597870700.0 - 4517590.878848932).abs() < 1e-6);
        assert!((mid.position.z * 149597870700.0 - 4487348.408865919).abs() < 1e-6);
        assert!(mid.velocity.dot(mid.position).abs() < 1e-20);
    }
    #[test]
    fn vector_subtraction_matches_exact_spherical_parallax_and_exposes_old_error() {
        for altitude in [10_f64, 45.0, 80.0] {
            let h = altitude.to_radians();
            let relative = Vector3 {
                x: 60.0 * h.cos(),
                y: 0.0,
                z: 60.0 * h.sin(),
            } - Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            let exact = relative.z.atan2(relative.x);
            assert_eq!(exact, (60.0 * h.sin() - 1.0).atan2(60.0 * h.cos()));
            let legacy = h - (1.0_f64 / 60.0).asin() * h.cos();
            let error = (legacy - exact).abs().to_degrees() * 3600.0;
            assert!(error > 9.0 && error < 29.0);
        }
    }
}
