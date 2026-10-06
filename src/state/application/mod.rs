//! Construct the complete root once the configuration and catalog are ready.
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
    /// Construct once from validated configuration and a loaded catalog; no catalog buffers are copied.
    pub fn new(config: Config, sky: Sky, timings: StepTimes) -> Self {
        let catalog = sky.catalog.clone();
        let run = RunState::new(sky, &config.cache);
        Self { config, catalog, run, timings }
    }
}
