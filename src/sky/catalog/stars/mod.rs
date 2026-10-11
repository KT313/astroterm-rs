//! Preparing catalog objects; shared records live in model.
mod color;
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
    let magnitude = entry.magnitude;
    Star {
        id: entry.id,
        name: entry.name,
        brightness_key: motion.brightest_magnitude(magnitude),
        motion,
        magnitude,
        display_color: color::classify_star_color(entry.spectral_type, entry.color_index),
        has_data: entry.has_data,
    }
}
