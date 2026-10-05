//! The root facade intentionally exposes the active-run storage owners; processing modules only borrow them.
//! Application ownership root: catalog, working buffers and diagnostics remain inspectable for the whole run.
//! The adjacent README.md maps each storage family to its producer, consumers, units and reset rules.
mod run;
pub mod rendering;
pub use rendering::RenderingState;
pub mod simulation;
pub mod observation;
pub mod projection;
pub mod scene;
pub use simulation::SimulationState;
pub use observation::ObservationCache;
pub use projection::ProjectionCache;
pub use scene::SceneCache;
pub use run::RunState;
#[cfg(feature = "memory-diagnostics")]
pub mod memory;

use std::sync::Arc;
use crate::{model::{config::Config, Sky, SkyCatalog}, timing::StepTimes};

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
