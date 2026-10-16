//! Speed groups: every star of a region belongs to one of `SPEED_GROUP_COUNT` groups by its proper motion, and
//! receives a configured TTL at startup: longest for the slowest group, shortest for the fastest.
//!
//! A star carries no rate, only the stored motion bound: the largest angle between its catalog direction and its
//! straight-line position at either end of the computational interval, plus the quantization margin. That angle is
//! compared with the displacement of each rate threshold over the shorter half of the interval. This is a
//! grouping policy, not a bound on angular speed at every date: approaching stars can move faster later.
//! `tests/stellar_hold_accuracy.rs` audits the resulting fixed holds without changing runtime behavior.
use crate::formulas::computational_years;
use crate::constants::{SPEED_GROUP_TTL_MS, SPEED_GROUP_BOUNDS, SPEED_GROUP_COUNT};
use crate::model::{Region, StarGroup, StarTable};

/// The group of a star from its stored motion bound (radians over the computational interval). Stars without
/// motion data have a bound of the quantization margin only and land in group 0, the slow group.
pub fn speed_group(motion_bound: f32) -> usize {
    let bound = f64::from(motion_bound);
    SPEED_GROUP_BOUNDS.iter().filter(|&&rate| drift_bound_for_rate(rate) <= bound).count()
}

/// Configured TTL in simulated milliseconds. Sorting the values assigns the longest to group 0 (slowest),
/// regardless of their order in the constants array. Runtime expiration reads the value stored in StarGroup.
pub fn get_ttl_of_speed_group_ms(group_index: usize) -> i64 {
    // Reject an index that does not identify a configured speed group.
    assert!(
        group_index < SPEED_GROUP_COUNT,
        "speed group {group_index} does not exist",
    );

    assert!(SPEED_GROUP_TTL_MS.iter().all(|&ttl_ms| ttl_ms >= 0), "speed-group TTLs must be nonnegative");

    // Copy the configured TTLs and put the longest first, matching the slow-to-fast group order.
    let mut ttls_slowest_to_fastest = SPEED_GROUP_TTL_MS;
    ttls_slowest_to_fastest.sort_by(|left_ttl, right_ttl| {
        right_ttl.cmp(left_ttl) // Comparing right against left puts larger TTLs first.
    });

    // Return this group's TTL in simulated milliseconds.
    ttls_slowest_to_fastest[group_index]
}

impl StarGroup {
    /// Build a group with its configured TTL and empty simulation/image caches.
    pub fn new(stars: StarTable, speed_group: usize) -> Self {
        Self { ttl_ms: get_ttl_of_speed_group_ms(speed_group), stars, ..Self::default() }
    }
}

impl Default for Region {
    fn default() -> Self {
        Self {
            speed_groups: std::array::from_fn(|group| StarGroup::new(StarTable::default(), group)),
            merged_tile: Default::default(),
        }
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
    fn configured_ttls_are_assigned_from_slowest_to_fastest() {
        let region = Region::default();
        let actual = region.speed_groups.map(|group| group.ttl_ms);
        assert_eq!(actual, [365 * 86_400_000, 10 * 86_400_000, 86_400_000]);
        assert!(actual.windows(2).all(|pair| pair[0] > pair[1]));
        for (group, ttl_ms) in actual.into_iter().enumerate() {
            assert_eq!(StarGroup::new(StarTable::default(), group).ttl_ms, ttl_ms);
        }
    }

    #[test]
    #[should_panic(expected = "speed group 3 does not exist")]
    fn hold_time_rejects_unknown_groups() {
        get_ttl_of_speed_group_ms(SPEED_GROUP_COUNT);
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
