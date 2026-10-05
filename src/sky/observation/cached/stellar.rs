//! Bounded passes over selected stars. Each pass has one timer per batch, never one per star.
//! The selected source indices are unique. Catalog data stays borrowed; scratch holds at most one batch.
use super::*;

const BATCH_SIZE: usize = 1024;

use crate::model::observation::{StellarWork, ValidityCounts};
#[cfg(test)]
use crate::model::observation::SelectedStar;

pub(super) fn update_stellar_motion(storage: crate::state::observation::StellarMotionBuffers<'_>, catalog_stars: &crate::model::storage::StarStorage, epoch: f64, times: &mut StepTimes) -> usize {
    times.with_memory(|times| {
        let step = times.active_memory_step();
        times.record_memory(step, || MemoryEvent::borrow(BufferId::WorkingStars, Access::ReadOnly, BufferShape::vector(storage.working.value(), IndexDomain::Working)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::StellarScratch, Access::Writable, BufferShape::vector(storage.scratch, IndexDomain::Working)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::StellarSamples, Access::Writable, BufferShape::unknown(IndexDomain::Catalog)));
        if let Some(classes) = storage.prepared_classes {
            times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogClassifications, Access::ReadOnly, BufferShape::slice(classes, IndexDomain::Catalog)));
        }
    });
    let maximum = storage.config.age_seconds(Group::StellarState);
    let reuse = storage.config.allows(Group::StellarState);
    let refresh = times.measure("Motion cache decision", || {
        storage.motion
            .needs_refresh(&storage.working.generation, epoch, Some(maximum), reuse)
    });
    times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::MotionSamples,
        if refresh { Operation::Refresh(storage.motion.stats.last_reason.expect("refresh reason")) } else { Operation::Reuse }));
    times.describe("Motion cache decision", || {
        format!(
            "refresh={refresh}; working generation={}; requested maximum={maximum} s",
            storage.working.generation
        )
    });
    if refresh {
        let years = years_since_j2000(epoch);
        let trajectories = catalog_stars.borrow_trajectory_fields();
        let mut values = times.measure("Motion output allocation", || {
            Vec::with_capacity(storage.working.value().len())
        });
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::MotionSamples, Operation::Reserve,
            None, Some(BufferShape::vector(&values, IndexDomain::Working)), Some(values.capacity()), None));
        times.describe("Motion output allocation", || {
            format!(
                "reserved results={}; element bytes={}",
                values.capacity(),
                std::mem::size_of::<(Vector3, f64)>()
            )
        });
        let mut singular_count = 0;
        let mut validity = maximum;
        let mut refreshed = 0;
        let mut reused = 0;
        let mut counts = ValidityCounts::default();
        let scratch = &mut *storage.scratch;
        let scratch_before = times.inspect_memory(|| BufferShape::vector(scratch, IndexDomain::Working));
        times.measure("Stellar scratch preparation", || {
            scratch.clear();
            scratch.reserve(BATCH_SIZE);
        });

        times.with_memory(|times| {
            times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Clear,
                scratch_before, None, scratch_before.and_then(|s| s.len), None));
            times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Reserve,
                scratch_before, Some(BufferShape::vector(scratch, IndexDomain::Working)), Some(BATCH_SIZE), None));
        });

        // Keep large data in its owners; only bounded intermediate samples cross these passes.
        times.measure_batches("Stellar batches", |batches| {
            for stars in storage.working.value().chunks(BATCH_SIZE) {
                let batch_before = batches.inspect_memory(|| (refreshed, reused, BufferShape::vector(scratch, IndexDomain::Working)));
                batches.measure("Stellar cache lookup and decisions", || {
                    scratch.clear();
                    for star in stars {
                        let entry = storage.stellar.entry(star.source_index).or_default();
                        let before = entry.stats;
                        let refresh = entry.needs_refresh(&(), epoch, Some(maximum), reuse);
                        storage.stats.hits += entry.stats.hits - before.hits;
                        storage.stats.bypasses += entry.stats.bypasses - before.bypasses;
                        refreshed += usize::from(refresh);
                        reused += usize::from(!refresh);
                        scratch.push(StellarWork {
                            source_index: star.source_index,
                            magnitude: catalog_stars.magnitude(star.source_index),
                            refresh,
                            motion: None,
                            class: None,
                            sample: (!refresh).then(|| *entry.value()),
                            valid_seconds: entry.valid_seconds,
                            calculated_at: entry.calculated_at.unwrap_or(epoch),
                        });
                    }
                });
                if let Some((_, previous_reuses, before)) = batch_before {
                    let step = batches.last_memory_step();
                    batches.record_memory(step, || MemoryEvent::borrow(BufferId::WorkingStars, Access::ReadOnly, BufferShape::slice(stars, IndexDomain::Working)));
                    batches.record_memory(step, || MemoryEvent::operation(BufferId::StellarScratch, Operation::Clear, Some(before), None, before.len, None));
                    batches.record_memory(step, || {
                        let shape = BufferShape::vector(scratch, IndexDomain::Working);
                        MemoryEvent::operation(BufferId::StellarScratch, Operation::Build, None, Some(shape), shape.len, shape.logical_bytes())
                    });
                    batches.record_memory(step, || MemoryEvent::operation(BufferId::StellarSamples, Operation::Reuse, None, None, Some(reused - previous_reuses), None));
                }
                batches.measure("Trajectory reads", || {
                    for item in scratch.iter_mut().filter(|s| s.refresh) {
                        let motion = trajectories.motion(item.source_index);
                        item.class = Some(
                            storage.prepared_classes
                                .map_or_else(|| motion.classify(), |classes| classes[item.source_index]),
                        );
                        item.motion = Some(motion);
                    }
                });
                if batch_before.is_some() {
                    batches.record_memory(batches.last_memory_step(), || MemoryEvent::borrow(BufferId::CatalogTrajectories, Access::ReadOnly, BufferShape::unknown(IndexDomain::Catalog)));
                }
                batches.measure("Motion and magnitude calculation", || {
                    for item in scratch.iter_mut().filter(|s| s.refresh) {
                        item.sample = Some(item.motion.unwrap().evaluate_classified(
                            years,
                            item.magnitude,
                            item.class.unwrap(),
                        ));
                    }
                });
                if let Some((previous_refreshes, _, _)) = batch_before {
                    batches.record_memory(batches.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Build, None, None, Some(refreshed - previous_refreshes), None));
                }
                batches.measure("Stellar validity qualification", || {
                    for item in scratch.iter_mut().filter(|s| s.refresh) {
                        item.valid_seconds = qualify_stellar_span_counted(
                            item.motion.unwrap(),
                            item.class.unwrap(),
                            item.sample.unwrap(),
                            epoch,
                            item.magnitude,
                            maximum,
                            &mut counts,
                        );
                        item.calculated_at = epoch;
                    }
                });
                if let Some((previous_refreshes, _, _)) = batch_before {
                    batches.record_memory(batches.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Write, None, None, Some(refreshed - previous_refreshes), None));
                }
                let mut store_counts = batches.inspect_memory(|| (0_usize, 0_usize));
                batches.measure("Stellar cache stores", || {
                    for item in scratch.iter().filter(|s| s.refresh) {
                        let entry = storage.stellar.get_mut(&item.source_index).expect("cache entry prepared");
                        let before = entry.stats.refreshes;
                        let outcome = entry.store((), epoch, item.valid_seconds, item.sample.unwrap());
                        if let Some((changed, equal)) = &mut store_counts {
                            if outcome.value_changed { *changed += 1; } else { *equal += 1; }
                        }
                        storage.stats.refreshes += entry.stats.refreshes - before;
                    }
                });
                if let Some((changed, equal)) = store_counts {
                    let step = batches.last_memory_step();
                    if changed + equal != 0 {
                        batches.record_memory(step, || MemoryEvent::unknown_operation(BufferId::StellarSamples, Operation::Compare));
                    }
                    for (count, value_changed) in [(changed, true), (equal, false)] {
                        if count != 0 {
                            batches.record_memory(step, || MemoryEvent::operation(BufferId::StellarSamples,
                                Operation::Store { value_changed }, None, None, Some(count), None));
                        }
                    }
                }
                let output_before = batches.inspect_memory(|| BufferShape::vector(&values, IndexDomain::Working));
                batches.measure("Motion output assembly", || {
                    for item in scratch.iter() {
                        validity = validity
                            .min((item.valid_seconds - (epoch - item.calculated_at).abs() * 86400.0).max(0.0));
                        let sample = item.sample.unwrap();
                        values.push((sample.direction, sample.magnitude));
                        singular_count += usize::from(sample.used_singular_fallback);
                    }
                });
                batches.record_memory(batches.last_memory_step(), || MemoryEvent::operation(BufferId::MotionSamples, Operation::Append,
                    output_before, Some(BufferShape::vector(&values, IndexDomain::Working)), Some(scratch.len()), scratch.len().checked_mul(std::mem::size_of::<(Vector3, f64)>())));
            }
        });
        times.describe("Stellar batches", || format!("input/output stars={}; refreshed={refreshed}; reused={reused}; batch limit={BATCH_SIZE}; scratch capacity={} records ({} bytes); refreshed samples with positive validity={}; zero validity: outside interval={}, singular={}, moving with distance={}, zero configured limit={}, boundary/angular bound={}; validity probe evaluations={}; resulting batch validity={validity} s", values.len(), scratch.capacity(), scratch.capacity()*std::mem::size_of::<StellarWork>(), counts.positive, counts.outside_interval, counts.singular, counts.moving_distance, counts.zero_limit, counts.boundary_or_bound, counts.probe_evaluations));
        let outcome = times.measure("Motion cache store", || {
            storage.motion
                .store(storage.working.generation, epoch, validity, (values, singular_count))
        });
        times.record_store(BufferId::MotionSamples, outcome);
        let scratch_before = times.inspect_memory(|| BufferShape::vector(scratch, IndexDomain::Working));
        times.measure("Stellar scratch clear", || scratch.clear());
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Clear,
            scratch_before, Some(BufferShape::vector(scratch, IndexDomain::Working)), scratch_before.and_then(|s| s.len), None));
    }
    storage.motion.value().1
}

/// Keep the same conservative rule and arithmetic as the original fused loop. Reasons are mutually exclusive.
fn qualify_stellar_span_counted(
    motion: StellarMotion,
    class: crate::astro::models::stars::StellarClass,
    sample: StellarSample,
    epoch: f64,
    magnitude: f64,
    maximum: f64,
    counts: &mut ValidityCounts,
) -> f64 {
    let interval = crate::astro::COMPUTATIONAL_INTERVAL;
    if !interval.contains(epoch) {
        counts.outside_interval += 1;
        return 0.0;
    }
    if sample.used_singular_fallback {
        counts.singular += 1;
        return 0.0;
    }
    if class.has_variable_brightness() {
        counts.moving_distance += 1;
        return 0.0;
    }
    let mut span = maximum
        .min((epoch - interval.start_tt) * 86400.0)
        .min((interval.end_tt - epoch) * 86400.0)
        .max(0.0);
    for _ in 0..32 {
        let valid = [-span, span].into_iter().all(|offset| {
            counts.probe_evaluations += 1;
            let end = motion.evaluate_classified(years_since_j2000(epoch + offset / 86400.0), magnitude, class);
            let angle = sample
                .direction
                .cross(end.direction)
                .length()
                .atan2(sample.direction.dot(end.direction));
            angle.is_finite() && angle <= 0.09_f64.to_radians() / 3600.0
        });
        if valid {
            if span > 0.0 {
                counts.positive += 1;
            } else if maximum == 0.0 {
                counts.zero_limit += 1;
            } else {
                counts.boundary_or_bound += 1;
            }
            return span;
        }
        span *= 0.5;
    }
    counts.boundary_or_bound += 1;
    0.0
}

#[cfg(test)]
pub(super) fn qualify_stellar_span(
    motion: StellarMotion,
    sample: StellarSample,
    epoch: f64,
    magnitude: f64,
    maximum: f64,
) -> f64 {
    qualify_stellar_span_counted(
        motion,
        motion.classify(),
        sample,
        epoch,
        magnitude,
        maximum,
        &mut ValidityCounts::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The previous per-star loop is deliberately retained as an independent ordering/storage reference.
    fn update_fused(cache: &mut ObservationCache, output: &mut ObservedSky, epoch: f64) {
        let maximum = cache.config.age_seconds(Group::StellarState);
        let reuse = cache.config.allows(Group::StellarState);
        if cache
            .motion
            .needs_refresh(&cache.working.generation, epoch, Some(maximum), reuse)
        {
            let trajectories = output.catalog.stars.borrow_trajectory_fields();
            let mut values = Vec::with_capacity(cache.working.value().len());
            let mut singular = 0;
            let mut validity = maximum;
            for star in cache.working.value() {
                let entry = cache.stellar.entry(star.source_index).or_default();
                let before = entry.stats;
                if entry.needs_refresh(&(), epoch, Some(maximum), reuse) {
                    let motion = trajectories.motion(star.source_index);
                    let sample = motion.evaluate(
                        years_since_j2000(epoch),
                        output.catalog.stars.magnitude(star.source_index),
                    );
                    entry.store(
                        (),
                        epoch,
                        qualify_stellar_span(
                            motion,
                            sample,
                            epoch,
                            output.catalog.stars.magnitude(star.source_index),
                            maximum,
                        ),
                        sample,
                    );
                }
                cache.stellar_stats.hits += entry.stats.hits - before.hits;
                cache.stellar_stats.refreshes += entry.stats.refreshes - before.refreshes;
                cache.stellar_stats.bypasses += entry.stats.bypasses - before.bypasses;
                validity = validity
                    .min((entry.valid_seconds - (epoch - entry.calculated_at.unwrap()).abs() * 86400.0).max(0.0));
                let sample = entry.value();
                values.push((sample.direction, sample.magnitude));
                singular += usize::from(sample.used_singular_fallback);
            }
            cache
                .motion
                .store(cache.working.generation, epoch, validity, (values, singular));
        }
        output.runtime_singular_count = cache.motion.value().1;
    }

    #[test]
    fn batches_match_fused_values_validity_and_cache_history() {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars[0].space_motion = Some(crate::catalog::SpaceMotion {
            distance_pc: 1.0,
            position: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            velocity: Vector3 {
                x: 0.0,
                y: 1e-7,
                z: 0.0,
            },
        });
        let base = crate::sky::create_sky_from_catalog(&parsed);
        assert!(base.stars.len() > BATCH_SIZE * 2);
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let mut fused = ObservationCache::new(config.clone());
            let mut batched = ObservationCache::new(config);
            let mut a = base.clone();
            let mut b = base.clone();
            let mut scratch_allocation = None;
            for (frame, offset) in [0.0, 0.0, 0.001, -0.001, 0.25, 0.25].into_iter().enumerate() {
                let epoch = crate::astro::J2000 + offset;
                let stars: Vec<_> = if frame < 4 {
                    base.stars
                        .iter()
                        .map(|s| SelectedStar {
                            source_index: s.source_index,
                            drawable: s.drawable,
                        })
                        .collect()
                } else {
                    base.stars
                        .iter()
                        .step_by(3)
                        .map(|s| SelectedStar {
                            source_index: s.source_index,
                            drawable: s.drawable,
                        })
                        .collect()
                };
                fused.working.store(frame as u64, epoch, 0.0, stars.clone());
                batched.working.store(frame as u64, epoch, 0.0, stars);
                update_fused(&mut fused, &mut a, epoch);
                let mut times = StepTimes::with_trace(true);
                b.runtime_singular_count = update_stellar_motion(batched.borrow_stellar_motion(), &b.catalog.stars, epoch, &mut times);
                assert!(batched.stellar_scratch.is_empty());
                assert!(batched.stellar_scratch.capacity() >= BATCH_SIZE);
                let allocation = (batched.stellar_scratch.as_ptr(), batched.stellar_scratch.capacity());
                if let Some(previous) = scratch_allocation {
                    assert_eq!(allocation, previous, "paused, changed time/membership and bypass reuse the same scratch allocation");
                }
                scratch_allocation = Some(allocation);
                assert_eq!(fused.motion, batched.motion);
                assert_eq!(fused.stellar, batched.stellar);
                assert_eq!(fused.stellar_stats, batched.stellar_stats);
                assert_eq!(a.runtime_singular_count, b.runtime_singular_count);
                assert!(
                    times.trace().unwrap().steps.len() < 20,
                    "trace size must not grow with star count"
                );
            }
        }
    }

    #[test]
    fn qualification_reasons_partition_refreshed_samples() {
        let base = StellarMotion {
            u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w: Vector3::default(),
            distance_pc: None,
        };
        let mut counts = ValidityCounts::default();
        for (motion, epoch, maximum) in [
            (base, crate::astro::J2000, 360.0),
            (base, crate::astro::J2000, 0.0),
            (base, crate::astro::COMPUTATIONAL_INTERVAL.end_tt, 360.0),
            (
                StellarMotion {
                    w: Vector3 {
                        x: 0.0,
                        y: 1e-4,
                        z: 0.0,
                    },
                    distance_pc: Some(1.0),
                    ..base
                },
                crate::astro::J2000,
                360.0,
            ),
        ] {
            let sample = motion.evaluate(years_since_j2000(epoch), 4.0);
            qualify_stellar_span_counted(motion, motion.classify(), sample, epoch, 4.0, maximum, &mut counts);
        }
        assert_eq!(
            (
                counts.positive,
                counts.zero_limit,
                counts.outside_interval,
                counts.moving_distance
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(counts.probe_evaluations, 4);
    }
}
