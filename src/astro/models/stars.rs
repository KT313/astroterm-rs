//! Linear angular proper motion in J2000. Input/output radians, TT, f64; no dependencies or observer corrections.
//! The legacy 365.2425-day year is preserved until phase 3. No persistent samples: evaluated at each frame epoch.
use crate::astro::{Equatorial, J2000};
/// Apply proper motion (radians per year) to a J2000 catalog position. The result is still in the J2000 frame.
/// Time is TT; the legacy 365.2425-day motion year is retained until the stellar-model replacement.
pub fn compute_star_position(catalog: Equatorial, proper_motion: Equatorial, julian_date_tt: f64) -> Equatorial {
    let years_since_j2000 = (julian_date_tt - J2000) / 365.2425;
    Equatorial {
        right_ascension: catalog.right_ascension + proper_motion.right_ascension * years_since_j2000,
        declination: catalog.declination + proper_motion.declination * years_since_j2000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_preserves_epoch_and_legacy_year() {
        let p = Equatorial {
            right_ascension: 1.0,
            declination: 0.5,
        };
        let v = Equatorial {
            right_ascension: 0.001,
            declination: -0.002,
        };
        assert_eq!(compute_star_position(p, v, J2000), p);
        let later = compute_star_position(p, v, J2000 + 365.2425);
        assert!((later.right_ascension - 1.001).abs() < 1e-12);
        assert!((later.declination - 0.498).abs() < 1e-12);
    }
}
