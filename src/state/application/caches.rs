//! Recomputed data, grouped by the stage that produces it. Processing borrows these independent fields; the
//! terminal guard is external.
use crate::{cache::CacheConfig, model::Sky, state::{ProjectionCache, ObservationCache, SimulationState, RenderingState}};

pub struct Caches {
    pub sky: Sky,                       // observed objects for the current frame
    pub simulation: SimulationState,    // planet, Moon and Earth-axis results
    pub observation: ObservationCache,  // star selection and apparent-direction caches
    pub projection: ProjectionCache,    // screen positions and draw order
    pub rendering: RenderingState,      // canvas, glyph and presenter buffers
}
impl Caches {
    pub(super) fn new(sky: Sky, config: &CacheConfig) -> Self {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(config);
        Self { sky, simulation, observation: ObservationCache::new(config.clone()), projection: ProjectionCache::new(config.clone()), rendering: RenderingState::Pending }
    }

}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Caches { sky, simulation, observation, projection, rendering });
