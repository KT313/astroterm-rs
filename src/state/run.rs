//! Active-run data, grouped by producer. Processing borrows these independent fields; the terminal guard is external.
use crate::{cache::CacheConfig, model::Sky, state::{ProjectionCache, ObservationCache, SimulationState, RenderingState}};

pub struct RunState {
    pub sky: Sky,
    pub simulation: SimulationState,
    pub observation: ObservationCache,
    pub projection: ProjectionCache,
    pub rendering: RenderingState,
}
impl RunState {
    pub(super) fn new(sky: Sky, config: &CacheConfig) -> Self {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(config);
        Self { sky, simulation, observation: ObservationCache::new(config.clone()), projection: ProjectionCache::new(config.clone()), rendering: RenderingState::Pending }
    }

}

#[cfg(feature = "memory-diagnostics")]
crate::cache::buffers::report_fields!(RunState { sky, simulation, observation, projection, rendering });
