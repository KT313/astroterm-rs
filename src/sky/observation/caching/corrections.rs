//! Cached body sampling and separate apparent, horizontal and refracted directions.
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
pub(in crate::sky::observation) fn update_aberration(
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

pub(in crate::sky::observation) fn update_horizon_rotation(
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

pub(in crate::sky::observation) fn update_refraction(
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

