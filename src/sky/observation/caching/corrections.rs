//! Cached body geometry; regional apparent inputs feed the retained horizontal/refraction snapshots.
use super::*;
use super::super::memory::{direction_shape, record_direction_commit};



pub(in crate::sky::observation) fn update_observer_subtraction(
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
                subtract_observer_position(bodies_cache.value(), observer, output);
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
            times.record_borrow(BufferId::BodySamples, Access::ReadOnly, || BufferShape {
                len: Some(bodies_cache.value().planets.len() + 1), capacity: None,
                element_bytes: Some(std::mem::size_of::<crate::astro::models::BodyState>()),
                domain: IndexDomain::Objects, quality: crate::cache::Quality::ExactPayload,
            }); // read the original planet and Moon samples without cloning their storage
        }
        times.record_memory(times.last_memory_step(), || {
            let count = output.planets.len() + 1;
            MemoryEvent::operation(BufferId::RelativeBodies, Operation::Copy, None, None, Some(count), count.checked_mul(std::mem::size_of::<Vector3>()))
        });
    }
}

pub(in crate::sky::observation) fn update_moon_illumination(illumination: &mut IlluminationCache, config: &CacheConfig, epoch: f64, output: &mut ObservedSky, times: &mut StepTimes) {
    let memory_before = times.inspect_memory(|| snapshot_cache(illumination));
    times.measure("Moon illumination", || {
        let value = illumination.get_or_update(
            (output.moon.position, output.sun().position),
            epoch,
            config.allows(Group::SolarSystemGeometry),
            || {
                super::super::stages::update_moon_illumination(output.moon.position, output.sun().position, output);
                (output.moon.illumination, output.moon.phase)
            },
        );
        (output.moon.illumination, output.moon.phase) = *value;
    });
    record_cache(times, BufferId::MoonIllumination, memory_before, illumination);
}

#[allow(clippy::too_many_arguments)]
pub(in crate::sky::observation) fn update_horizon_rotation(
    apparent: crate::state::ApparentDirections<'_>, sources: &mut crate::state::HorizontalSources, horizontal: &mut HorizontalCache,
    work: &mut Directions, config: &CacheConfig, epoch: f64, rotation: crate::astro::Matrix3, times: &mut StepTimes,
) {
    times.measure_steps("Horizon rotation", |times| {
        update_horizontal_sources(apparent, sources, times);
        let key = (sources.revision, rotation);
        let refresh = times.measure("Horizontal cache decision", || horizontal.needs_refresh(&key, epoch, None, config.allows(Group::HorizontalSky)));
        times.record_candidate_decision(times.last_memory_step(), BufferId::HorizontalDirections, BufferId::HorizontalDirections, refresh, horizontal.stats.last_reason);
        if !refresh { return; } // the completed directions are already the output; no restoration pass
        prepare_direction_work(work, apparent.star_count(), apparent.bodies().0.len(), BufferId::HorizontalWork, times);
        times.measure("Horizon rotation calculation", || {
            for (region, directions) in apparent.regions() {
                assert_eq!(region.start, work.0.len(), "apparent regions must cover output in order");
                work.0.extend(directions.iter().map(|&direction| rotation.apply(direction)));
            }
            let (planets, moon) = apparent.bodies();
            work.1.extend(planets.iter().map(|&direction| rotation.apply(direction).normalized()));
            work.2 = rotation.apply(moon); // preserve the Moon vector length
        });
        times.record_borrow(BufferId::RegionalApparent, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Observed));
        times.record_borrow(BufferId::BodyApparentDirections, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Objects));
        times.record_shape(BufferId::HorizontalWork, Operation::Build, None, || direction_shape(work));
        let completed = times.inspect_memory(|| direction_shape(work));
        let displaced = times.inspect_memory(|| horizontal.stored().map(direction_shape)).flatten();
        let outcome = times.measure("Direction cache store", || horizontal.store_reusing_pair(key, epoch, 0.0, work));
        record_direction_commit(times, BufferId::HorizontalDirections, BufferId::HorizontalWork, completed, displaced, work, outcome);
    });
}

#[allow(clippy::too_many_arguments)]
pub(in crate::sky::observation) fn update_refraction(horizontal: &HorizontalCache, refracted: &mut Cache<(u64, bool), Directions>, work: &mut Directions,
    config: &CacheConfig, epoch: f64, enabled: bool, times: &mut StepTimes,
) {
    if !enabled { return; }
    times.measure_steps("Refraction", |times| {
        let key = (horizontal.generation, true);
        let refresh = times.measure("Refraction cache decision", || refracted.needs_refresh(&key, epoch, None, config.allows(Group::Refraction)));
        times.record_candidate_decision(times.last_memory_step(), BufferId::RefractedDirections, BufferId::RefractedDirections, refresh, refracted.stats.last_reason);
        if !refresh { return; }
        let input = horizontal.value();
        prepare_direction_work(work, input.0.len(), input.1.len(), BufferId::RefractionWork, times);
        times.measure("Refraction calculation", || {
            work.0.extend(input.0.iter().copied().map(crate::astro::refract_direction));
            work.1.extend(input.1.iter().copied().map(crate::astro::refract_direction));
            work.2 = crate::astro::refract_direction(input.2);
        });
        times.record_borrow(BufferId::HorizontalDirections, Access::ReadOnly, || direction_shape(input));
        times.record_shape(BufferId::RefractionWork, Operation::Build, None, || direction_shape(work));
        let completed = times.inspect_memory(|| direction_shape(work));
        let displaced = times.inspect_memory(|| refracted.stored().map(direction_shape)).flatten();
        let outcome = times.measure("Direction cache store", || refracted.store_reusing_pair(key, epoch, 0.0, work));
        record_direction_commit(times, BufferId::RefractedDirections, BufferId::RefractionWork, completed, displaced, work, outcome);
    });
}

fn prepare_direction_work(work: &mut Directions, stars: usize, planets: usize, buffer: BufferId, times: &mut StepTimes) {
    let before = times.inspect_memory(|| direction_shape(work));
    times.measure("Direction work preparation", || {
        work.0.clear(); work.1.clear();
        work.0.reserve(stars); work.1.reserve(planets);
    }); // allocations persist after commit; interrupted work is discarded before retry
    times.record_shape(buffer, Operation::Reserve, before, || direction_shape(work));
}

fn update_horizontal_sources(apparent: crate::state::ApparentDirections<'_>, sources: &mut crate::state::HorizontalSources, times: &mut StepTimes) {
    let before = times.inspect_memory(|| BufferShape::vector(&sources.regions, IndexDomain::Regions));
    let changed = times.measure("Horizontal request preparation", || {
        let body_generation = Some(apparent.body_generation());
        if sources.body_generation == body_generation && apparent.dependencies().eq(sources.regions.iter().copied()) { return false; }
        sources.regions.clear();
        sources.regions.extend(apparent.dependencies()); // retain only ordered spans and versions, with reusable capacity
        sources.body_generation = body_generation;
        sources.revision = sources.revision.checked_add(1).expect("horizontal request revision exhausted");
        true
    });
    times.record_shape(BufferId::HorizontalRequest, if changed { Operation::Build } else { Operation::Reuse }, before,
        || BufferShape::vector(&sources.regions, IndexDomain::Regions));
}
