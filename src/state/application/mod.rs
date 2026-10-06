//! Construct the complete root from validated configuration; the prepared catalog is installed afterwards.
mod run;
pub use run::RunState;

use std::sync::Arc;
use crate::model::{Config, Sky, SkyCatalog};
use crate::timing::StepTimes;

pub struct ApplicationState {
    pub config: Config,
    pub catalog: Arc<SkyCatalog>,
    pub run: RunState,
    pub timings: StepTimes,
}
impl ApplicationState {
    /// Construct once from validated configuration. Every owner exists with its final type; the catalog starts
    /// empty and `replace_catalog` installs the loaded one before the frame loop.
    pub fn new(config: Config, timings: StepTimes) -> Self {
        let catalog = Arc::new(SkyCatalog::empty());
        let run = RunState::new(Sky::new(catalog.clone()), &config.cache);
        Self { config, catalog, run, timings }
    }

    /// Install the prepared catalog: the only catalog mutation, done once at startup. The observed sky is rebuilt
    /// from it (name and figure copies); observation caches are still empty and `prepare_frame_data` runs later.
    /// The same `Arc` is shared by `catalog` and `run.sky.catalog`, so no catalog buffers are copied.
    pub fn replace_catalog(&mut self, catalog: Arc<SkyCatalog>) {
        self.catalog = catalog.clone();
        self.run.sky = Sky::new(catalog);
    }
}
