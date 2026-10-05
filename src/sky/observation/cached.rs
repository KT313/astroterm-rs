//! Observation orchestration over state-owned buffers. Each correction retains its own output; no corrected vector becomes a model input.
mod diagnostics;
mod stellar;
use super::memory::{snapshot_cache, record_cache, record_observer_memory, record_direction_pass, record_direction_capture, record_direction_restoration};
use crate::timing::memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use crate::state::ObservationCache;
use crate::state::observation::{
    RegionCache, CandidateCache, SelectedCache, WorkingCache, MotionCache, EligibleCache,
    RelativeCache, IlluminationCache, ApparentCache, HorizontalCache, ObserverBuffers, LightTimeBuffers,
};
use super::{stages::*, *};
use crate::astro::models::stars::{StellarMotion, StellarSample, years_since_j2000};
use crate::cache::{Cache, CacheConfig, Group};
use crate::model::{ObservedStar, SkyCatalog};
use std::sync::Arc;

#[cfg(test)]
use stellar::qualify_stellar_span;

use crate::model::observation::{Directions, BodyKey, CorrectionSelection, BodySamples};

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
        let observer = super::prepare_observer(simulation, time, site)?;
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
        super::prepare_light_time_samples(simulation, observer, times)?;
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

/// Resolve all fallible body dependencies first; correction passes then publish a complete sky.
#[allow(clippy::too_many_arguments)]
pub fn observe_cached_sky(
    storage: &mut ObservationCache,
    simulation: &SimulationState,
    observer: &ObserverState,
    threshold: f64,
    refraction: bool,
    region: crate::model::SkyRegion,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    if storage
        .catalog
        .as_ref()
        .is_none_or(|catalog| !Arc::ptr_eq(catalog, &output.catalog))
    {
        let observer_cache = std::mem::take(&mut storage.observer);
        let light_time_cache = std::mem::take(&mut storage.light_time);
        *storage = ObservationCache::new(storage.config.clone());
        storage.observer = observer_cache;
        storage.light_time = light_time_cache;
        storage.catalog = Some(output.catalog.clone());
    }
    let mut previous_reports = None;
    times.measure_diagnostics(|_| previous_reports = Some(storage.reports()));
    let epoch = observer.time.tt;

    // candidate membership is independent from intrinsic stellar cache lifetimes
    update_region_filtering(&mut storage.region, &storage.config, epoch, refraction, region, observer, &output.catalog.grid, times);
    update_brightness_bounds(&mut storage.candidates, &storage.region, &storage.config, epoch, threshold, &output.catalog, times);
    update_body_sampling(&mut storage.bodies, &storage.config, epoch, observer, simulation, times)?;

    output.selection = storage.candidates.value().1;
    update_candidate_validation(&mut storage.selected, &storage.candidates, &storage.config, epoch, threshold, &output.catalog, times);
    update_constellation_endpoints(&mut storage.working, &storage.selected, &storage.config, epoch, &output.catalog.endpoint_indices, times);
    times.measure_steps("Stellar motion", |times| {
        output.runtime_singular_count = stellar::update_stellar_motion(storage.borrow_stellar_motion(), &output.catalog.stars, epoch, times);
    });
    update_current_brightness(&mut storage.eligible, &storage.working, &storage.motion, &storage.config, epoch, threshold, &mut output.magnitude_threshold, times);

    update_correction_selection(&storage.working, &storage.eligible, &mut storage.corrections, &storage.motion, &storage.config, epoch, output, times);

    // each cache owns a distinct coordinate-space result
    update_observer_subtraction(&mut storage.relative, &storage.bodies, &storage.config, epoch, observer, output, times);
    update_moon_illumination(&mut storage.illumination, &storage.config, epoch, output, times);
    update_aberration(&storage.motion, &storage.relative, &storage.corrections, &mut storage.apparent, &storage.config, epoch, observer, output, times);
    update_horizon_rotation(&storage.apparent, &mut storage.horizontal, &storage.config, epoch, observer, output, times);
    update_refraction(&storage.horizontal, &mut storage.refracted, &storage.config, epoch, refraction && observer.atmosphere, output, times);
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(epoch);
    times.measure_diagnostics(|times| {
        diagnostics::describe_observation(storage, output, threshold, times);
        if let Some(previous) = previous_reports {
            for (before, after) in previous.into_iter().zip(storage.reports()).skip(2) {
                times.describe(after.name, || format!("cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; stored TT={:?}; validity={} s", after.stats.hits - before.stats.hits, after.stats.refreshes - before.stats.refreshes, after.stats.bypasses - before.stats.bypasses, after.stats.last_reason, after.calculated_at, after.valid_seconds));
            }
        }
    });
    Ok(())
}


#[allow(clippy::too_many_arguments)]
fn update_region_filtering(
    region_cache: &mut RegionCache, config: &CacheConfig, epoch: f64, refraction: bool,
    region: crate::model::SkyRegion, observer: &ObserverState, grid: &crate::model::SkyGrid, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(region_cache));
    times.measure("Region filtering", || {
        region_cache.get_or_update(
            (region, *observer, refraction),
            epoch,
            config.allows(Group::CandidateSelection),
            || crate::sky::grid::select_region(grid, region, observer, refraction && observer.atmosphere),
        );
    });
    record_cache(times, BufferId::RegionSelection, memory_before, region_cache);
    times.record_borrow(BufferId::CatalogGrid, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
}

fn update_brightness_bounds(
    candidates: &mut CandidateCache, region_cache: &RegionCache, config: &CacheConfig, epoch: f64, threshold: f64,
    catalog: &SkyCatalog, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(candidates));
    times.measure("Brightness bounds", || {
        candidates.get_or_update(
            (region_cache.generation, threshold),
            epoch,
            config.allows(Group::CandidateSelection),
            || {
                let mut indices = Vec::new();
                let stats = crate::sky::grid::select_brightness(&catalog.grid, &catalog.stars,
                    region_cache.value(),
                    threshold,
                    &mut indices);
                (indices, stats)
            },
        );
    });
    record_cache(times, BufferId::BrightnessCandidates, memory_before, candidates);
    if memory_before.is_some_and(|(_, stats)| candidates.stats.refreshes != stats.refreshes) {
        times.record_borrow(BufferId::CatalogStars, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_borrow(BufferId::RegionSelection, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_build(BufferId::BrightnessCandidates, || BufferShape::vector(&candidates.value().0, IndexDomain::Catalog));
    }
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

fn update_candidate_validation(
    selected_cache: &mut SelectedCache, candidates: &CandidateCache, config: &CacheConfig, epoch: f64, threshold: f64,
    catalog: &SkyCatalog, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(selected_cache));
    times.measure("Candidate validation", || {
        selected_cache.get_or_update(
            (
                candidates.generation,
                crate::astro::COMPUTATIONAL_INTERVAL.contains(epoch),
            ),
            epoch,
            config.allows(Group::WorkingSet),
            || filter_brightness_candidates(catalog, epoch, threshold, Some(&candidates.value().0)),
        );
    });
    record_cache(times, BufferId::ValidatedCandidates, memory_before, selected_cache);
    if memory_before.is_some_and(|(_, stats)| selected_cache.stats.refreshes != stats.refreshes) {
        times.record_borrow(BufferId::BrightnessCandidates, Access::ReadOnly, || BufferShape::vector(&candidates.value().0, IndexDomain::Catalog));
        times.record_build(BufferId::ValidatedCandidates, || BufferShape::vector(selected_cache.value(), IndexDomain::Catalog));
    }
}

fn update_constellation_endpoints(
    working_cache: &mut WorkingCache, selected_cache: &SelectedCache, config: &CacheConfig, epoch: f64,
    endpoints: &[usize], times: &mut StepTimes,
) {
    times.measure_steps("Constellation endpoints", |times| {
        let refresh = times.measure("Working-set cache decision", || {
            working_cache
                .needs_refresh(&selected_cache.generation, epoch, None, config.allows(Group::WorkingSet))
        });
        times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::WorkingStars,
            if refresh { Operation::Refresh(working_cache.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
        if refresh {
            let selected = times.measure("Selected index copy", || selected_cache.value().clone());
            times.record_memory(times.last_memory_step(), || {
                let shape = BufferShape::vector(&selected, IndexDomain::Catalog);
                MemoryEvent::operation(BufferId::ValidatedCandidates, Operation::Copy, None, Some(shape), shape.len, shape.logical_bytes())
            });
            times.describe("Selected index copy", || {
                format!(
                    "copied indices={}; bytes={}",
                    selected.len(),
                    selected.len() * std::mem::size_of::<usize>()
                )
            });
            let working = merge_constellation_endpoints(selected, endpoints, times);
            let outcome = times.measure("Working-set cache store", || {
                working_cache.store(selected_cache.generation, epoch, 0.0, working)
            });
            times.record_store(BufferId::WorkingStars, outcome);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn update_current_brightness(
    eligible: &mut EligibleCache, working_cache: &WorkingCache, motion: &MotionCache, config: &CacheConfig,
    epoch: f64, threshold: f64, magnitude_threshold: &mut f64, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(eligible));
    times.measure("Current brightness", || {
        eligible.get_or_update(
            (working_cache.generation, motion.generation, threshold),
            epoch,
            config.allows(Group::StellarVisibility),
            || {
                working_cache
                    .value()
                    .iter()
                    .zip(&motion.value().0)
                    .map(|(star, &(_, magnitude))| star.drawable && magnitude <= threshold)
                    .collect()
            },
        );
        *magnitude_threshold = threshold;
    });
    record_cache(times, BufferId::VisibilityFlags, memory_before, eligible);
    if memory_before.is_some_and(|(_, stats)| eligible.stats.refreshes != stats.refreshes) {
        times.record_borrow(BufferId::WorkingStars, Access::ReadOnly, || BufferShape::vector(working_cache.value(), IndexDomain::Working));
        times.record_borrow(BufferId::MotionSamples, Access::ReadOnly, || BufferShape::vector(&motion.value().0, IndexDomain::Working));
        times.record_build(BufferId::VisibilityFlags, || BufferShape::vector(eligible.value(), IndexDomain::Working));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_correction_selection(
    working_cache: &WorkingCache, eligible: &EligibleCache, corrections: &mut Cache<(u64, u64), CorrectionSelection>,
    motion: &MotionCache, config: &CacheConfig, epoch: f64, output: &mut ObservedSky, times: &mut StepTimes,
) {
    times.measure_steps("Correction selection", |times| {
        let key = (working_cache.generation, eligible.generation);
        let refresh = times.measure("Correction cache decision", || {
            corrections
                .needs_refresh(&key, epoch, None, config.allows(Group::StellarVisibility))
        });
        times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::CorrectionSelection,
            if refresh { Operation::Refresh(corrections.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
        if refresh {
            let (indices, stats) = times.measure("Correction index selection", || {
                select_correction_indices(
                    working_cache.value().iter().map(|s| s.source_index),
                    eligible.value(),
                    &output.catalog.endpoint_indices,
                )
            });
            let outcome = times.measure("Correction cache store", || {
                corrections
                    .store(key, epoch, 0.0, CorrectionSelection { indices, stats })
            });
            times.record_store(BufferId::CorrectionSelection, outcome);
        }
        let selection = corrections.value();
        let output_before = times.inspect_memory(|| BufferShape::vector(&output.stars, IndexDomain::Observed));
        times.measure("Corrected-star buffer construction", || {
            let working = working_cache.value();
            let drawable = eligible.value();
            let samples = &motion.value().0;
            output.stars.clear();
            output.stars.extend(selection.indices.iter().map(|&index| ObservedStar {
                source_index: working[index].source_index,
                drawable: drawable[index],
                position: samples[index].0,
                magnitude: samples[index].1,
            }));
            output.corrections = selection.stats;
        });
        {
            times.record_borrow(BufferId::CorrectionSelection, Access::ReadOnly, || BufferShape::vector(&selection.indices, IndexDomain::Working));
            times.record_borrow(BufferId::MotionSamples, Access::ReadOnly, || BufferShape::vector(&motion.value().0, IndexDomain::Working));
            times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::ObservedStars, Operation::Clear, output_before, None, output_before.and_then(|s| s.len), None));
            times.record_build(BufferId::ObservedStars, || BufferShape::vector(&output.stars, IndexDomain::Observed));
        }
        times.describe("Corrected-star buffer construction", || {
            format!(
                "output records={}; estimated record bytes={}; calculated state only; catalog metadata copied=0",
                output.stars.len(),
                output.stars.len() * std::mem::size_of::<ObservedStar>()
            )
        });
    });
}

fn update_observer_subtraction(
    relative_cache: &mut RelativeCache, bodies_cache: &Cache<BodyKey, BodySamples>, config: &CacheConfig, epoch: f64,
    observer: &ObserverState, output: &mut ObservedSky, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(relative_cache));
    times.measure("Observer subtraction", || {
        let relative = relative_cache.get_or_update(
            (bodies_cache.generation, observer.state),
            epoch,
            config.allows(Group::SolarSystemGeometry),
            || {
                subtract_observer_position(bodies_cache.value().clone(), observer, output);
                (
                    output.planets.iter().map(|p| p.position).collect(),
                    output.moon.position,
                )
            },
        );
        for (planet, &position) in output.planets.iter_mut().zip(&relative.0) {
            planet.position = position;
        }
        output.moon.position = relative.1;
    });
    record_cache(times, BufferId::RelativeBodies, memory_before, relative_cache);
    {
        if memory_before.is_some_and(|(_, stats)| relative_cache.stats.refreshes != stats.refreshes) {
            times.record_memory(times.last_memory_step(), || {
                let count = bodies_cache.value().planets.len() + 1;
                MemoryEvent::operation(BufferId::BodySamples, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<crate::astro::models::BodyState>()))
            });
        }
        times.record_memory(times.last_memory_step(), || {
            let count = output.planets.len() + 1;
            MemoryEvent::operation(BufferId::RelativeBodies, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<Vector3>()))
        });
    }
}

fn update_moon_illumination(illumination: &mut IlluminationCache, config: &CacheConfig, epoch: f64, output: &mut ObservedSky, times: &mut StepTimes) {
    let memory_before = times.inspect_memory(|| snapshot_cache(illumination));
    times.measure("Moon illumination", || {
        let value = illumination.get_or_update(
            (output.moon.position, output.sun().position),
            epoch,
            config.allows(Group::SolarSystemGeometry),
            || {
                super::stages::update_moon_illumination(output.moon.position, output.sun().position, output);
                (output.moon.illumination, output.moon.phase)
            },
        );
        (output.moon.illumination, output.moon.phase) = *value;
    });
    record_cache(times, BufferId::MoonIllumination, memory_before, illumination);
}

#[allow(clippy::too_many_arguments)]
fn update_aberration(
    motion: &MotionCache, relative_cache: &RelativeCache, corrections: &Cache<(u64, u64), CorrectionSelection>,
    apparent: &mut ApparentCache, config: &CacheConfig, epoch: f64, observer: &ObserverState,
    output: &mut ObservedSky, times: &mut StepTimes,
) {
    times.measure_steps("Aberration", |times| {
        let key = (
            motion.generation,
            relative_cache.generation,
            corrections.generation,
            observer.state.velocity,
        );
        let refresh = times.measure("Apparent cache decision", || {
            apparent
                .needs_refresh(&key, epoch, None, config.allows(Group::ApparentDirections))
        });
        times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::ApparentDirections,
            if refresh { Operation::Refresh(apparent.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
        if refresh {
            times.measure("Aberration calculation", || {
                apply_sky_aberration(observer.state.velocity, output)
            });
            record_direction_pass(times, output);
            let positions = times.measure("Direction capture", || capture_directions(output));
            record_direction_capture(times, BufferId::ApparentDirections, &positions);
            let outcome = times.measure("Direction cache store", || {
                apparent.store(key, epoch, 0.0, positions)
            });
            times.record_store(BufferId::ApparentDirections, outcome);
        } else {
            times.measure("Direction restoration", || {
                restore_directions(output, apparent.value())
            });
            record_direction_restoration(times, BufferId::ApparentDirections, output);
        }
    });
}

fn update_horizon_rotation(
    apparent: &ApparentCache, horizontal: &mut HorizontalCache, config: &CacheConfig, epoch: f64,
    observer: &ObserverState, output: &mut ObservedSky, times: &mut StepTimes,
) {
    times.measure_steps("Horizon rotation", |times| {
        let key = (apparent.generation, observer.inertial_to_horizon);
        let refresh = times.measure("Horizontal cache decision", || {
            horizontal
                .needs_refresh(&key, epoch, None, config.allows(Group::HorizontalSky))
        });
        times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::HorizontalDirections,
            if refresh { Operation::Refresh(horizontal.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
        if refresh {
            times.measure("Horizon rotation calculation", || {
                rotate_sky_to_horizon(observer.inertial_to_horizon, output)
            });
            record_direction_pass(times, output);
            let positions = times.measure("Direction capture", || capture_directions(output));
            record_direction_capture(times, BufferId::HorizontalDirections, &positions);
            let outcome = times.measure("Direction cache store", || {
                horizontal.store(key, epoch, 0.0, positions)
            });
            times.record_store(BufferId::HorizontalDirections, outcome);
        } else {
            times.measure("Direction restoration", || {
                restore_directions(output, horizontal.value())
            });
            record_direction_restoration(times, BufferId::HorizontalDirections, output);
        }
    });
}

fn update_refraction(
    horizontal: &HorizontalCache, refracted: &mut Cache<(u64, bool), Directions>, config: &CacheConfig, epoch: f64,
    enabled: bool, output: &mut ObservedSky, times: &mut StepTimes,
) {
    output.refracted = false;
    if enabled {
        times.measure_steps("Refraction", |times| {
            let key = (horizontal.generation, true);
            let refresh = times.measure("Refraction cache decision", || {
                refracted
                    .needs_refresh(&key, epoch, None, config.allows(Group::Refraction))
            });
            times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::RefractedDirections,
                if refresh { Operation::Refresh(refracted.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
            if refresh {
                times.measure("Refraction calculation", || refract_sky_positions(output));
                record_direction_pass(times, output);
                let positions = times.measure("Direction capture", || capture_directions(output));
                record_direction_capture(times, BufferId::RefractedDirections, &positions);
                let outcome = times.measure("Direction cache store", || {
                    refracted.store(key, epoch, 0.0, positions)
                });
                times.record_store(BufferId::RefractedDirections, outcome);
            } else {
                times.measure("Direction restoration", || {
                    restore_directions(output, refracted.value())
                });
                record_direction_restoration(times, BufferId::RefractedDirections, output);
            }
            output.refracted = true;
        });
    }
}

fn capture_directions(sky: &ObservedSky) -> Directions {
    (
        sky.stars.iter().map(|s| s.position).collect(),
        sky.planets.iter().map(|p| p.position).collect(),
        sky.moon.position,
    )
}
fn restore_directions(sky: &mut ObservedSky, directions: &Directions) {
    for (star, &p) in sky.stars.iter_mut().zip(&directions.0) {
        star.position = p;
    }
    for (planet, &p) in sky.planets.iter_mut().zip(&directions.1) {
        planet.position = p;
    }
    sky.moon.position = directions.2;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small_catalog() -> Arc<SkyCatalog> {
        let mut source = crate::catalog::load_embedded_catalog().unwrap();
        source.stars.retain(|star| star.has_data);
        source.stars.truncate(6);
        Arc::new(crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(source.stars, Default::default(), vec![])))
    }

    #[test]
    fn catalog_replacement_retains_geometry_and_resets_catalog_state() {
        let first = small_catalog();
        let replacement = Arc::new((*first).clone()); // equal content, distinct identity must still reset catalog-indexed caches
        let mut storage = ObservationCache::default();
        let mut simulation = SimulationState::default();
        let time = FrameTime::from_utc(crate::astro::J2000);
        let mut times = StepTimes::default();
        prepare_observation_catalog(&mut storage, first.clone(), &mut times);
        crate::sky::update_simulation(&mut simulation, time, &[], &mut times).unwrap();
        let mut observer = prepare_cached_observer(&mut storage, &simulation, time, Observer::default()).unwrap();
        prepare_cached_light_time(&mut storage, &mut simulation, &mut observer, &mut times).unwrap();
        let mut sky = ObservedSky::new(first);
        observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
        let expected = sky.clone();
        let saved_observer = storage.observer.clone();
        let saved_light_time = storage.light_time.clone();
        assert!(storage.prepared_classes.is_some());
        assert_eq!(storage.stellar.len(), 6);

        sky = ObservedSky::new(replacement.clone());
        observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
        assert!(Arc::ptr_eq(storage.catalog.as_ref().unwrap(), &replacement));
        assert!(storage.prepared_classes.is_none()); // automatic replacement keeps the existing classify-on-demand policy
        assert_eq!(storage.observer, saved_observer);
        assert_eq!(storage.light_time, saved_light_time);
        assert_eq!(storage.motion.stats.refreshes, 1);
        assert_eq!(storage.stellar_stats.refreshes, 6);
        assert_eq!(sky.stars, expected.stars);
        assert_eq!(sky.planets, expected.planets);
        assert_eq!(sky.moon, expected.moon);
    }

    #[test]
    fn missing_body_coverage_keeps_committed_body_cache_and_published_sky() {
        let catalog = small_catalog();
        let mut storage = ObservationCache::default();
        let mut simulation = SimulationState::default();
        let time = FrameTime::from_utc(crate::astro::J2000);
        let mut times = StepTimes::default();
        crate::sky::update_simulation(&mut simulation, time, &[], &mut times).unwrap();
        let observer = crate::sky::prepare_observation(&mut simulation, time, Observer::default()).unwrap();
        let mut sky = ObservedSky::new(catalog);
        observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
        let previous_sky = sky.clone();
        let previous_bodies = storage.bodies.clone();
        let mut missing = observer;
        missing.emission_tt[0] -= 10.0;
        assert!(observe_cached_sky(&mut storage, &simulation, &missing, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).is_err());
        assert_eq!(storage.bodies.generation, previous_bodies.generation);
        assert_eq!(storage.bodies.stats.refreshes, previous_bodies.stats.refreshes);
        let mut expected_failure = previous_bodies.clone();
        expected_failure.has_been_invalidated = true;
        expected_failure.stats.last_reason = Some(crate::cache::RefreshReason::Dependencies);
        assert!(storage.bodies == expected_failure); // invalidated prior key/value remain owned and cannot be read until refreshed
        assert_eq!(sky, previous_sky);
        observe_cached_sky(&mut storage, &simulation, &observer, 20.0, true, crate::model::SkyRegion::All, &mut sky, &mut times).unwrap();
        assert_eq!(sky, previous_sky);
        assert_eq!(storage.bodies.generation, previous_bodies.generation);
    }

    #[test]
    fn stellar_hold_bound_covers_forward_reverse_and_fast_motion() {
        let epoch = crate::astro::J2000;
        for speed in [0.0, 0.01, 10.0, 10000.0] {
            let motion = StellarMotion {
                u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
                w: Vector3 {
                    x: 0.0,
                    y: speed,
                    z: 0.0,
                },
                distance_pc: None,
            };
            let sample = motion.evaluate(0.0, 5.0);
            let span = qualify_stellar_span(motion, sample, epoch, 5.0, 360.0);
            for fraction in [-1.0, -0.3, 0.0, 0.4, 1.0] {
                let direct = motion.evaluate(years_since_j2000(epoch + span * fraction / 86400.0), 5.0);
                let error = sample
                    .direction
                    .cross(direct.direction)
                    .length()
                    .atan2(sample.direction.dot(direct.direction));
                assert!(error.to_degrees() * 3600.0 <= 0.1);
                assert_eq!(direct.magnitude, sample.magnitude);
            }
        }
    }
    #[test]
    fn variable_brightness_and_out_of_range_states_use_exact_epochs() {
        let motion = StellarMotion {
            u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w: Vector3 {
                x: -0.01,
                y: 0.001,
                z: 0.0,
            },
            distance_pc: Some(1.0),
        };
        let sample = motion.evaluate(0.0, 5.0);
        assert_eq!(
            qualify_stellar_span(motion, sample, crate::astro::J2000, 5.0, 360.0),
            0.0
        );
        assert_eq!(
            qualify_stellar_span(motion, sample, crate::astro::COMPUTATIONAL_INTERVAL.end_tt, 5.0, 360.0),
            0.0
        );
    }
}
