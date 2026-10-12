//! Step 4 of the frame: `refresh_regions`. Selected regions keep up-to-date group images and region images
//! (plan section 6). This module holds the speed-group rule (chunk 4) and the per-group simulation caches
//! (chunk 9); the images and the step itself follow in chunk 10.
mod groups;
mod simulation;
pub use groups::{speed_group, speed_group_hold_seconds, drift_bound_for_rate};
pub use simulation::{GroupSimulation, Simulations, Refresh, UNPLACED, rows_wanted};
