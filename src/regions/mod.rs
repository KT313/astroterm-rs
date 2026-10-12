//! Step 4 of the frame: `refresh_regions`. Selected regions keep up-to-date group images and region images
//! (plan section 6). Until the step itself is written (chunk 9), this module holds the speed-group rule.
mod groups;
pub use groups::{speed_group, speed_group_hold_seconds, drift_bound_for_rate};
