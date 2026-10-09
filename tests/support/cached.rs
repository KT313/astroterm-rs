//! Test-only orchestration over the same separate owners and stages used by the application.
#![allow(dead_code)]
use astroterm::{cache::{CacheConfig, CacheStats, CacheReport}, model::{SkyCatalog, FrameTime, ObserverState, SimulationError, ObservedSky, SkyRegion}, state::{ObserverPreparationCache, StarSelectionCache, StellarSimulationState, ObservationCache, SimulationState}, timing::StepTimes, astro::Observer};
use std::sync::Arc;

#[derive(Default)]
pub struct PipelineCache {
    pub observer: ObserverPreparationCache,
    pub selection: StarSelectionCache,
    pub stars: StellarSimulationState,
    pub observation: ObservationCache,
}
impl PipelineCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { observer: ObserverPreparationCache::new(config.clone()), selection: StarSelectionCache::new(config.clone()), stars: StellarSimulationState::new(config.clone()), observation: ObservationCache::new(config) }
    }
    pub fn invalidate_view(&mut self) { self.selection.invalidate_view(); }
    pub fn region_report(&self, index: usize) -> Option<CacheReport> { self.stars.region_report(index) }
    pub fn stats(&self) -> CacheStats {
        let mut total = CacheStats::default();
        for s in [self.observer.stats(), self.selection.stats(), self.stars.stats(), self.observation.stats()] {
            total.hits += s.hits; total.refreshes += s.refreshes; total.bypasses += s.bypasses;
        }
        total
    }
    pub fn reports(&self) -> Vec<CacheReport> {
        self.observer.reports().into_iter().chain(self.selection.reports()).chain(self.observation.reports()).collect()
    }
}
pub fn prepare_stellar_catalog(storage: &mut PipelineCache, catalog: Arc<SkyCatalog>, times: &mut StepTimes) {
    astroterm::sky::prepare_stellar_catalog(&mut storage.stars, catalog, astroterm::astro::J2000, times);
}
pub fn prepare_cached_observer(storage: &mut PipelineCache, simulation: &SimulationState, time: FrameTime, site: Observer) -> Result<ObserverState, SimulationError> {
    astroterm::sky::prepare_cached_observer(&mut storage.observer, simulation, time, site)
}
pub fn prepare_cached_light_time(storage: &mut PipelineCache, simulation: &mut SimulationState, observer: &mut ObserverState, times: &mut StepTimes) -> Result<(), SimulationError> {
    astroterm::sky::prepare_cached_light_time(&mut storage.observer, simulation, observer, times)
}
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_sky(storage: &mut PipelineCache, simulation: &SimulationState, observer: &ObserverState, threshold: f64,
    refraction: bool, region: SkyRegion, output: &mut ObservedSky, times: &mut StepTimes) -> Result<(), SimulationError> {
    astroterm::sky::prepare_cached_bodies(&mut storage.observer, simulation, observer, times)?;
    times.measure_steps("Star selection", |times| astroterm::sky::select_cached_stars(&mut storage.selection, &output.catalog, observer, threshold, refraction, region, times));
    times.measure_steps("Stellar simulation", |times| astroterm::sky::simulate_stars(&mut storage.stars, storage.selection.stars(), observer.time.tt, times));
    times.measure_steps("Observation", |times| astroterm::sky::observe_cached_sky(&mut storage.observation, storage.stars.results(storage.selection.stars()), storage.observer.bodies(observer), observer, threshold, refraction, output, times));
    Ok(())
}

pub fn prepare_frame(storage: &mut PipelineCache, simulation: &mut SimulationState, time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<ObserverState, SimulationError> {
    astroterm::sky::begin_solar_system_frame(simulation, &mut storage.observer, time, site, times)?;
    astroterm::sky::prepare_observer_inputs(&mut storage.observer, simulation, time, site, times)
}
