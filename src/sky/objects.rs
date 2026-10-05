//! Preparing catalog objects; shared records live in model.
use crate::model::Star;
use crate::catalog::CatalogStar;
use crate::astro::{Equatorial, models::stars::StellarMotion};

/// Convert one catalog row into unquantized motion and metadata; storage applies precision and singular policies.
pub fn prepare_star(entry: &CatalogStar) -> Star {
    let direction = Equatorial {
        right_ascension: entry.right_ascension,
        declination: entry.declination,
    };
    let motion = entry.space_motion.map_or_else(
        || StellarMotion::from_sky_motion(direction, entry.ra_motion_cos_dec, entry.dec_motion),
        |space| StellarMotion::from_direction_velocity(direction, space.distance_pc, space.velocity),
    );
    let singular_fallback = false; // storage applies the policy after quantization
    let magnitude = f64::from(entry.magnitude);
    Star {
        id: entry.id,
        name: entry.name,
        designation: entry.designation,
        brightness_key: motion.brightest_magnitude(magnitude),
        motion_bound: motion.motion_bound(),
        motion,
        magnitude,
        singular_fallback,
        spectral_type: entry.spectral_type,
        color_index: entry.color_index,
        has_data: entry.has_data,
    }
}
