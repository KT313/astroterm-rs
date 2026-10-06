//! Observer-relative sky preparation. The cached frame order is in pipeline.rs; direct evaluation is also available.
mod pipeline;
mod caching;
mod direct;
mod stages;
mod memory;

pub use pipeline::observe_cached_sky;
pub use caching::{prepare_observation_catalog, prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time};
pub use direct::{compose_observer_state, prepare_observer, prepare_light_time_samples, prepare_observation, observe_sky, observe_sky_candidates};
pub(crate) use direct::LIGHT_SPEED_AU_DAY;
use direct::{apply_aberration, apply_unit_aberration, body_id};
