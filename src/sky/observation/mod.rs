//! Observer-relative sky preparation. The cached frame order is in pipeline.rs; direct evaluation is also available.
mod pipeline;
mod caching;
#[path = "processing/aberration.rs"] mod direct;
#[path = "processing/direct.rs"] mod processing;
pub(crate) use processing::apply_direct_observation;
#[path = "processing/stages.rs"] mod stages;
#[path = "diagnostics/mod.rs"] mod memory;

pub use pipeline::{observe_cached_sky, observe_cached_regions};
use direct::{apply_aberration, apply_unit_aberration};
