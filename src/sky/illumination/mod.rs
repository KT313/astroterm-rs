//! Moon illumination: the formulas moved to `astro::models::moons` (pipeline restructuring, chunk 7); this module
//! re-exports them for the old pipeline until the cut-over.
pub use crate::astro::{compute_moon_illumination, name_moon_phase};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{MoonPhase, Vector3};
    #[test]
    fn full_new_quarter_and_waxing_use_the_angle_at_the_moon() {
        let moon = Vector3 { x: 1.0, y: 0.0, z: 0.0 };
        let north = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let full = compute_moon_illumination(moon, -moon, north);
        let new = compute_moon_illumination(moon, moon * 2.0, north);
        let quarter = compute_moon_illumination(
            moon,
            Vector3 {
                x: 1.0,
                y: -1.0,
                z: 0.0,
            },
            north,
        );
        let waning = compute_moon_illumination(moon, Vector3 { x: 1.0, y: 1.0, z: 0.0 }, north);
        assert_eq!((full.illuminated_fraction, crate::sky::name_moon_phase(full)), (1.0, MoonPhase::Full));
        assert_eq!((new.illuminated_fraction, crate::sky::name_moon_phase(new)), (0.0, MoonPhase::New));
        assert_eq!(quarter.illuminated_fraction, 0.5);
        assert_eq!(crate::sky::name_moon_phase(quarter), MoonPhase::FirstQuarter);
        assert!(quarter.waxing && !waning.waxing);
        assert_eq!(crate::sky::name_moon_phase(waning), MoonPhase::LastQuarter);
    }
}
