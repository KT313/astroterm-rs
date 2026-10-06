//! Construct the complete root from validated configuration; the prepared catalog is installed afterwards.
//!
//! The root has three groups, named after how long their data lives:
//! - `persistent`: loaded once at startup and never changed during a run (the prepared catalog),
//! - `cache`: everything recomputed from the persistent data and the simulated time (observed objects, caches, buffers),
//! - `timings`: diagnostics, not used for rendering.
mod caches;
pub use caches::Caches;

use std::sync::Arc;
use crate::model::{Config, Sky, SkyCatalog};
use crate::timing::StepTimes;

pub struct ApplicationState {
    pub config: Config,
    pub persistent: Persistent,
    pub cache: Caches,
    pub timings: StepTimes,
}

/// Data loaded once and shared read-only by every stage for the whole run.
pub struct Persistent {
    pub catalog: Arc<SkyCatalog>,
}

impl ApplicationState {
    /// Construct once from validated configuration. Every owner exists with its final type; the catalog starts
    /// empty and `replace_catalog` installs the loaded one before the frame loop.
    pub fn new(config: Config, timings: StepTimes) -> Self {
        let catalog = Arc::new(SkyCatalog::empty());
        let cache = Caches::new(Sky::new(catalog.clone()), &config.cache);
        Self { config, persistent: Persistent { catalog }, cache, timings }
    }

    /// Install the prepared catalog: the only catalog mutation, done once at startup. The observed sky is rebuilt
    /// from it (name and figure copies); observation caches are still empty and `prepare_frame_data` runs later.
    /// The same `Arc` is shared by `persistent.catalog` and `cache.sky.catalog`, so no catalog buffers are copied.
    pub fn replace_catalog(&mut self, catalog: Arc<SkyCatalog>) {
        self.persistent.catalog = catalog.clone();
        self.cache.sky = Sky::new(catalog);
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Persistent { catalog });
