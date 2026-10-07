//! Prepare the viewer and all observer-dependent solar-system samples before star work.
mod pipeline;
mod diagnostics;
pub use pipeline::prepare_observer_inputs;
#[path = "preparation/direct.rs"] mod direct;
#[path = "preparation/cached.rs"] mod cached;
pub use direct::{compose_observer_state, prepare_observer, prepare_light_time_samples, prepare_observation};
pub(crate) use direct::sample_body_states;
pub use cached::{prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time, prepare_cached_bodies};
