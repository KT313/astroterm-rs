//! Catalog and observer preparation plus cache reset boundaries.
use super::*;

/// Prepare catalog-only classifications once; replacing the catalog drops these with all dependent caches.
pub fn prepare_observation_catalog(storage: &mut ObservationCache, catalog: Arc<SkyCatalog>, times: &mut StepTimes) {
    *storage = ObservationCache::new(storage.config.clone());
    let classes: Vec<_> = times.measure("Stellar classifications", || {
        let trajectories = catalog.stars.borrow_trajectory_fields();
        (0..catalog.stars.len())
            .map(|i| trajectories.motion(i).classify())
            .collect()
    });
    {
        times.record_borrow(BufferId::CatalogTrajectories, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_build(BufferId::CatalogClassifications, || BufferShape::vector(&classes, IndexDomain::Catalog));
    }
    times.describe("Stellar classifications", || {
        format!(
            "stars={}; stationary={}; moving with distance={}; classification bytes={}",
            classes.len(),
            classes.iter().filter(|c| c.is_stationary()).count(),
            classes.iter().filter(|c| c.has_variable_brightness()).count(),
            classes.len() * std::mem::size_of::<crate::astro::models::stars::StellarClass>()
        )
    });
    storage.prepared_classes = Some(classes);
    storage.catalog = Some(catalog);
}

pub fn prepare_cached_observer(
    storage: &mut ObservationCache,
    simulation: &SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    update_cached_observer(storage.borrow_observer(), simulation, time, site)
}

/// Time observer preparation and inspect only its own cache, keeping diagnostics beside the domain step.
pub fn prepare_cached_observer_with_times(storage: &mut ObservationCache, simulation: &SimulationState, time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<ObserverState, SimulationError> {
    let before = times.inspect_memory(|| storage.observer_report());
    let result = times.measure("Observer geometry", || prepare_cached_observer(storage, simulation, time, site));
    if let Some(before) = before {
        let after = times.inspect_memory(|| storage.observer_report()).unwrap();
        record_observer_memory(times, &before, &after, storage.config.allows(Group::ObserverState));
    }
    result
}

fn update_cached_observer(
    storage: ObserverBuffers<'_>, simulation: &SimulationState, time: FrameTime, site: Observer,
) -> Result<ObserverState, SimulationError> {
    let key = (
        time,
        site,
        simulation.model_versions(),
        simulation.refresh_counts.planets,
        simulation.refresh_counts.orientation,
    );
    if storage
        .observer
        .needs_refresh(&key, time.tt, None, storage.config.allows(Group::ObserverState))
    {
        let observer = crate::sky::prepare_observer(simulation, time, site)?;
        storage.observer.store(key, time.tt, 0.0, observer);
    }
    Ok(*storage.observer.value())
}

pub fn prepare_cached_light_time(
    storage: &mut ObservationCache,
    simulation: &mut SimulationState,
    observer: &mut ObserverState,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    update_cached_light_time(storage.borrow_light_time(), simulation, observer, times)
}

fn update_cached_light_time(
    storage: LightTimeBuffers<'_>, simulation: &mut SimulationState, observer: &mut ObserverState,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let step = times.active_memory_step();
    let key = (*observer, simulation.model_versions());
    // A disabled model family must still receive frame-local emission coverage on a paused frame.
    let enabled = [
        Group::SolarSystemObservation,
        Group::PlanetarySamples,
        Group::LunarSamples,
    ]
    .into_iter()
    .all(|g| storage.config.allows(g));
    if storage.light_time.needs_refresh(&key, observer.time.tt, None, enabled) {
        times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::EmissionTimes, Operation::Refresh(storage.light_time.stats.last_reason.expect("refresh reason"))));
        crate::sky::prepare_light_time_samples(simulation, observer, times)?;
        let outcome = storage.light_time.store(key, observer.time.tt, 0.0, *observer);
        {
            times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::EmissionTimes, Operation::Compare));
            times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::EmissionTimes, Operation::Store { value_changed: outcome.value_changed }));
        }
    } else {
        *observer = *storage.light_time.value();
        times.record_memory(step, || MemoryEvent::unknown_operation(BufferId::EmissionTimes, Operation::Reuse));
    }
    Ok(())
}

pub(in crate::sky::observation) fn reset_catalog_if_changed(storage: &mut ObservationCache, requested_catalog: &Arc<SkyCatalog>) {
    if storage
        .catalog
        .as_ref()
        .is_none_or(|catalog| !Arc::ptr_eq(catalog, requested_catalog))
    {
        let observer_cache = std::mem::take(&mut storage.observer);
        let light_time_cache = std::mem::take(&mut storage.light_time);
        *storage = ObservationCache::new(storage.config.clone());
        storage.observer = observer_cache;
        storage.light_time = light_time_cache;
        storage.catalog = Some(requested_catalog.clone());
    }
}

