//! Speed groups: every star of a region belongs to one of `SPEED_GROUP_COUNT` groups by its proper motion, and
//! each group holds its computed directions for `HELD_DIRECTION_ERROR / upper motion bound` (plan section 5).
//!
//! A star carries no rate, only the stored motion bound: the largest angle between its catalog direction and its
//! straight-line position at either end of the computational interval, plus the quantization margin. The rate is
//! recovered from that angle over the shorter half of the interval, which never underestimates the present
//! tangential rate (a star is closest to the Sun, and so fastest, at the end where its drift is largest), so a star
//! is never placed in a group whose hold time is too long for it.
use crate::astro::{JULIAN_YEAR_DAYS, models::stars::computational_years};
use crate::constants::{FAST_GROUP_HOLD_SECONDS, HELD_DIRECTION_ERROR, SPEED_GROUP_BOUNDS, SPEED_GROUP_COUNT};

/// The group of a star from its stored motion bound (radians over the computational interval). Stars without
/// motion data have a bound of the quantization margin only and land in group 0, the slow group.
pub fn speed_group(motion_bound: f32) -> usize {
    let bound = f64::from(motion_bound);
    SPEED_GROUP_BOUNDS.iter().filter(|&&rate| drift_bound_for_rate(rate) <= bound).count()
}

/// Simulated seconds for which a group's computed directions stay within `HELD_DIRECTION_ERROR`: the error limit
/// divided by the group's upper rate, with today's fixed 10 days for the fast group.
pub fn speed_group_hold_seconds(group: usize) -> f64 {
    assert!(group < SPEED_GROUP_COUNT, "speed group {group} does not exist");
    match SPEED_GROUP_BOUNDS.get(group) {
        Some(upper_rate) => HELD_DIRECTION_ERROR / upper_rate * JULIAN_YEAR_DAYS * 86_400.0,
        None => FAST_GROUP_HOLD_SECONDS,
    }
}

/// The stored motion bound of a star moving at `rate` radians per Julian year across the sky: the angle it drifts
/// over the shorter half of the computational interval. Rates and bounds are compared through this one function.
pub fn drift_bound_for_rate(rate: f64) -> f64 {
    let (start, end) = computational_years();
    (rate * (-start).min(end)).atan()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::QUANTIZATION_MARGIN;

    const ARCSECOND: f64 = std::f64::consts::PI / (180.0 * 3600.0);

    fn bound_of(arcseconds_per_year: f64) -> f32 {
        drift_bound_for_rate(arcseconds_per_year * ARCSECOND) as f32
    }

    #[test]
    fn groups_split_at_the_bounds() {
        assert_eq!(SPEED_GROUP_COUNT, 3);
        assert_eq!(speed_group(bound_of(0.01)), 0);
        assert_eq!(speed_group(bound_of(0.03).next_down()), 0);
        assert_eq!(speed_group(bound_of(0.03)), 1);                     // a bound is the first rate of its group
        assert_eq!(speed_group(bound_of(0.1)), 1);
        assert_eq!(speed_group(bound_of(0.3).next_down()), 1);
        assert_eq!(speed_group(bound_of(0.3)), 2);
        assert_eq!(speed_group(bound_of(10.39)), 2);                    // Barnard's Star
    }

    #[test]
    fn stationary_and_motionless_stars_are_slow() {
        assert_eq!(speed_group(0.0), 0);
        assert_eq!(speed_group((QUANTIZATION_MARGIN as f32).next_up()), 0); // the loader's bound for a star without motion data
        assert_eq!(speed_group(bound_of(0.03) * 0.5), 0);
    }

    #[test]
    fn hold_times_follow_the_error_limit() {
        let year = JULIAN_YEAR_DAYS * 86_400.0;
        assert!((speed_group_hold_seconds(0) - 9.5 * year).abs() < 1e-3);
        assert!((speed_group_hold_seconds(1) - 0.95 * year).abs() < 1e-3);
        assert_eq!(speed_group_hold_seconds(2), 10.0 * 86_400.0);
        for (group, upper_rate) in SPEED_GROUP_BOUNDS.iter().enumerate() {   // held error at the upper rate is exactly the limit
            let error = upper_rate * speed_group_hold_seconds(group) / year;
            assert!((error - HELD_DIRECTION_ERROR).abs() < 1e-12);
        }
    }

    #[test]
    #[should_panic(expected = "speed group 3 does not exist")]
    fn hold_time_rejects_unknown_groups() {
        speed_group_hold_seconds(SPEED_GROUP_COUNT);
    }

    #[test]
    fn drift_bound_uses_the_shorter_half_of_the_interval() {
        let (start, end) = computational_years();
        let shorter = (-start).min(end);
        assert!(shorter > 9_000.0 && shorter < 10_100.0);
        assert_eq!(drift_bound_for_rate(0.0), 0.0);
        let one = drift_bound_for_rate(ARCSECOND);
        assert!((one - (ARCSECOND * shorter).atan()).abs() < 1e-15);
        assert!(one < ARCSECOND * shorter);                                 // the angle never exceeds the linear drift
    }
}
