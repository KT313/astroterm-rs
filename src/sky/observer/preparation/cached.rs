//! Independent reception, light-time and final body caches.
use crate::state::{ObserverPreparationCache, SimulationState, ObserverBuffers, LightTimeBuffers};
use crate::model::{FrameTime, ObserverState, SimulationError, BodySamples, ObservationBodyKey as BodyKey};
use crate::astro::Observer;
use crate::cache::{Cache, CacheConfig, Group};
use crate::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use crate::sky::{sample_body_states, record_observer_memory, snapshot_cache, record_cache};
pub fn prepare_cached_observer(
    storage: &mut ObserverPreparationCache,
    simulation: &SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    update_cached_observer(storage.borrow_observer(), simulation, time, site)
}

/// Time observer preparation and inspect only its own cache, keeping diagnostics beside the domain step.
pub fn prepare_cached_observer_with_times(storage: &mut ObserverPreparationCache, simulation: &SimulationState, time: FrameTime, site: Observer, times: &mut StepTimes) -> Result<ObserverState, SimulationError> {
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
    storage: &mut ObserverPreparationCache,
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


pub fn prepare_cached_bodies(storage: &mut ObserverPreparationCache, simulation: &SimulationState, observer: &ObserverState, times: &mut StepTimes) -> Result<(), SimulationError> {
    let previous = times.trace().map(|_| [storage.bodies.report("Body sampling")]);
    update_body_sampling(&mut storage.bodies, &storage.config, observer.time.tt, observer, simulation, times)?;
    times.describe("Body sampling", || format!("requested Sun/planets={}; Moon=1; output states={} at emission epochs", storage.bodies.value().planets.len(), storage.bodies.value().planets.len() + 1));
    crate::sky::describe_cache_reports(previous, || [storage.bodies.report("Body sampling")], times);
    Ok(())
}
fn update_body_sampling(
    bodies_cache: &mut Cache<BodyKey, BodySamples>, config: &CacheConfig, epoch: f64, observer: &ObserverState,
    simulation: &SimulationState, times: &mut StepTimes,
) -> Result<(), SimulationError> {
    let body_key = (
        *observer,
        simulation.refresh_counts.planets,
        simulation.refresh_counts.moon,
    );
    let memory_before = times.inspect_memory(|| snapshot_cache(bodies_cache));
    let result = times.measure("Body sampling", || -> Result<(), SimulationError> {
        if bodies_cache
            .needs_refresh(&body_key, epoch, None, config.allows(Group::SolarSystemObservation))
        {
            let bodies = sample_body_states(simulation, observer)?;
            bodies_cache.store(body_key, epoch, 0.0, bodies);
        }
        Ok(())
    });
    {
        record_cache(times, BufferId::BodySamples, memory_before, bodies_cache);
        times.record_borrow(BufferId::PlanetSamples, Access::ReadOnly, || BufferShape::vector(&simulation.planets, IndexDomain::ModelSamples));
        times.record_borrow(BufferId::LunarSamples, Access::ReadOnly, || BufferShape::vector(&simulation.moon, IndexDomain::ModelSamples));
    }
    result
}
