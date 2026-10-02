//! Validated AT-HYG space-motion inputs. The current sky still uses angular proper motion only.

use crate::astro::{Equatorial, Vector3};

/// J2000 equatorial position in parsecs and velocity in parsecs per Julian year.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceMotion {
    pub position: Vector3,
    pub velocity: Vector3,
}

/// km/s to pc/Julian year (365.25 days, IAU parsec).
const KM_S_TO_PC_YEAR: f64 = 365.25 * 86400.0 / 3.085677581491367e13;

/// Accept a reliable distance, or leave the star on its angular-only path. Missing position triples are rebuilt
/// from RA/Dec and distance. Missing velocity triples use tangential motion and an optional radial component;
/// absent radial velocity means zero. Supplied velocity triples in km/s take precedence over proper motion.
pub(super) fn prepare_space_motion(
    distance: Option<f64>,
    position: Option<Vector3>,
    velocity: Option<Vector3>,
    direction: Equatorial,
    motion_on_sky: Equatorial,
    radial_velocity: Option<f64>,
) -> Option<SpaceMotion> {
    let distance = distance.filter(|d| *d > 0.0 && *d < 100_000.0)?;
    if let Some(p) = position {
        let length = p.x.hypot(p.y).hypot(p.z);
        if (length - distance).abs() > distance * 0.01 {
            return None;
        }
    }
    let position = position.unwrap_or_else(|| direction.to_unit_vector() * distance);
    let velocity = velocity.map(|v| v * KM_S_TO_PC_YEAR).unwrap_or_else(|| {
        let unit = direction.to_unit_vector();
        let (sin_ra, cos_ra) = direction.right_ascension.sin_cos();
        let (sin_dec, cos_dec) = direction.declination.sin_cos();
        let east = Vector3 {
            x: -sin_ra,
            y: cos_ra,
            z: 0.0,
        };
        let north = Vector3 {
            x: -sin_dec * cos_ra,
            y: -sin_dec * sin_ra,
            z: cos_dec,
        };
        east * (motion_on_sky.right_ascension * distance)
            + north * (motion_on_sky.declination * distance)
            + unit * (radial_velocity.unwrap_or(0.0) * KM_S_TO_PC_YEAR)
    });
    Some(SpaceMotion { position, velocity })
}
