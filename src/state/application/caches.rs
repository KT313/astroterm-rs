//! Recomputed data, grouped by the stage that produces it. Processing borrows these independent fields; the
//! terminal guard is external.
use crate::{cache::CacheConfig, model::Sky, state::{ProjectionCache, ObservationCache, SimulationState, RenderingState}};

pub struct Caches {
    pub selection: crate::state::StarSelectionCache,
    pub observer: crate::state::ObserverPreparationCache,
    pub sky: Sky,                       // observed objects for the current frame
    pub simulation: crate::state::SimulationCaches,    // independent solar-system and stellar model results
    pub observation: ObservationCache,  // current eligibility and apparent-direction corrections
    pub projection: ProjectionCache,    // screen positions and draw order
    pub rendering: RenderingState,      // canvas, glyph and presenter buffers
}
impl Caches {
    /// Historical Obs counter includes selection, intrinsic star samples, observer preparation and corrections.
    pub fn sky_processing_stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for stats in [self.observer.stats(), self.selection.stats(), self.simulation.stars.stats(), self.observation.stats()] {
            total.hits += stats.hits; total.refreshes += stats.refreshes; total.bypasses += stats.bypasses;
        }
        total
    }

    pub(super) fn new(sky: Sky, config: &CacheConfig) -> Self {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(config);
        Self { selection: crate::state::StarSelectionCache::new(config.clone()), observer: crate::state::ObserverPreparationCache::new(config.clone()), sky, simulation: crate::state::SimulationCaches { solar_system: simulation, stars: crate::state::StellarSimulationState::new(config.clone()) }, observation: ObservationCache::new(config.clone()), projection: ProjectionCache::new(config.clone()), rendering: RenderingState::Pending }
    }

}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Caches { selection, observer, sky, simulation, observation, projection, rendering });
