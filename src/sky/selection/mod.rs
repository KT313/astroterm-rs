//! Select conservative catalog candidates before stellar simulation.
mod pipeline;
mod caching;
mod processing;
pub use pipeline::select_cached_stars;
pub(crate) use processing::{filter_brightness_candidates, merge_constellation_endpoints};
