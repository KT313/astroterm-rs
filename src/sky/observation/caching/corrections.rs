//! Cached body geometry: observer-relative positions, Moon lighting and the bodies' horizontal/refracted directions.
use super::*;

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

/// Only the Sun, planets and Moon get stored horizontal directions. Stars keep their regional apparent
/// directions; the view rotates them on read and projection folds the rotation into the camera axes.
pub(in crate::sky::observation) fn update_horizon_rotation(bodies: &BodyApparentCache, horizontal: &mut HorizontalCache, config: &CacheConfig, epoch: f64, rotation: crate::astro::Matrix3, times: &mut StepTimes) {
    times.measure_steps("Horizon rotation", |times| {
        let key = (bodies.generation, rotation);
        let refresh = times.measure("Horizontal cache decision", || horizontal.needs_refresh(&key, epoch, None, config.allows(Group::HorizontalSky)));
        times.record_candidate_decision(times.last_memory_step(), BufferId::HorizontalDirections, BufferId::HorizontalDirections, refresh, horizontal.stats.last_reason);
        if !refresh { return; }
        let (planets, moon) = bodies.value();
        let directions = times.measure("Horizon rotation calculation", || {
            (planets.iter().map(|&direction| rotation.apply(direction).normalized()).collect::<Vec<_>>(), rotation.apply(*moon)) // preserve the Moon vector length
        });
        times.record_borrow(BufferId::BodyApparentDirections, Access::ReadOnly, || BufferShape::slice(planets, IndexDomain::Objects));
        times.record_build(BufferId::HorizontalDirections, || body_direction_shape(&directions));
        let outcome = times.measure("Direction cache store", || horizontal.store(key, epoch, 0.0, directions));
        times.record_store(BufferId::HorizontalDirections, outcome);
    });
}

pub(in crate::sky::observation) fn update_refraction(horizontal: &HorizontalCache, refracted: &mut Cache<(u64, bool), BodyDirections>, config: &CacheConfig, epoch: f64, enabled: bool, times: &mut StepTimes) {
    if !enabled { return; }
    times.measure_steps("Refraction", |times| {
        let key = (horizontal.generation, true);
        let refresh = times.measure("Refraction cache decision", || refracted.needs_refresh(&key, epoch, None, config.allows(Group::Refraction)));
        times.record_candidate_decision(times.last_memory_step(), BufferId::RefractedDirections, BufferId::RefractedDirections, refresh, refracted.stats.last_reason);
        if !refresh { return; }
        let (planets, moon) = horizontal.value();
        let directions = times.measure("Refraction calculation", || {
            (planets.iter().copied().map(crate::astro::refract_direction).collect::<Vec<_>>(), crate::astro::refract_direction(*moon))
        });
        times.record_borrow(BufferId::HorizontalDirections, Access::ReadOnly, || body_direction_shape(horizontal.value()));
        times.record_build(BufferId::RefractedDirections, || body_direction_shape(&directions));
        let outcome = times.measure("Direction cache store", || refracted.store(key, epoch, 0.0, directions));
        times.record_store(BufferId::RefractedDirections, outcome);
    });
}

fn body_direction_shape(directions: &BodyDirections) -> BufferShape {
    BufferShape { len: Some(directions.0.len() + 1), capacity: Some(directions.0.capacity() + 1),
        element_bytes: Some(std::mem::size_of::<Vector3>()), domain: IndexDomain::Objects, quality: crate::cache::Quality::ExactPayload }
} // planet vector plus the inline Moon value
