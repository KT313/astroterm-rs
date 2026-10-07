//! Complete regional intrinsic samples, then selected-row output for observation.
mod pipeline;
mod processing;
#[path = "processing/diagnostics.rs"] mod diagnostics;
#[path = "processing/preparation.rs"] mod preparation;
#[path = "processing/direct.rs"] mod direct;
pub use pipeline::simulate_stars;
pub use preparation::prepare_stellar_catalog;
pub(crate) use direct::simulate_stars_direct;
