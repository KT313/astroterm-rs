//! Bounded passes over selected stars. Each pass has one timer per batch, never one per star.
//! The selected source indices are unique. Catalog data stays borrowed; scratch holds at most one batch.
use super::*;

const BATCH_SIZE: usize = 1024;

struct StellarWork {
    source_index: usize,
    magnitude: f64,
    refresh: bool,
    motion: Option<StellarMotion>,
    sample: Option<StellarSample>,
    valid_seconds: f64,
    calculated_at: f64,
}

#[derive(Default)]
struct ValidityCounts {
    positive: usize,
    outside_interval: usize,
    singular: usize,
    moving_distance: usize,
    zero_limit: usize,
    boundary_or_bound: usize,
    probe_evaluations: usize,
}

impl ObservationCache {
    pub(super) fn update_stellar_motion(&mut self, output: &mut ObservedSky, epoch: f64, times: &mut StepTimes) {
        let maximum = self.config.age_seconds(Group::StellarState);
        let reuse = self.config.allows(Group::StellarState);
        let refresh = times.measure("Motion cache decision", || {
            self.motion
                .needs_refresh(&self.working.generation, epoch, Some(maximum), reuse)
        });
        times.describe("Motion cache decision", || {
            format!(
                "refresh={refresh}; working generation={}; requested maximum={maximum} s",
                self.working.generation
            )
        });
        if refresh {
            let years = years_since_j2000(epoch);
            let trajectories = output.catalog.stars.borrow_trajectory_fields();
            let mut values = times.measure("Motion output allocation", || {
                Vec::with_capacity(self.working.value().len())
            });
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
            let mut scratch = times.measure("Stellar scratch allocation", || Vec::with_capacity(BATCH_SIZE));

            // Keep large data in its owners; only bounded intermediate samples cross these passes.
            times.measure_batches("Stellar batches", |batches| {
                for stars in self.working.value().chunks(BATCH_SIZE) {
                    batches.measure("Stellar cache lookup and decisions", || {
                        scratch.clear();
                        for star in stars {
                            let entry = self.stellar.entry(star.source_index).or_default();
                            let before = entry.stats;
                            let refresh = entry.needs_refresh(&(), epoch, Some(maximum), reuse);
                            self.stellar_stats.hits += entry.stats.hits - before.hits;
                            self.stellar_stats.bypasses += entry.stats.bypasses - before.bypasses;
                            refreshed += usize::from(refresh);
                            reused += usize::from(!refresh);
                            scratch.push(StellarWork {
                                source_index: star.source_index,
                                magnitude: output.catalog.stars.magnitude(star.source_index),
                                refresh,
                                motion: None,
                                sample: (!refresh).then(|| *entry.value()),
                                valid_seconds: entry.valid_seconds,
                                calculated_at: entry.calculated_at.unwrap_or(epoch),
                            });
                        }
                    });
                    batches.measure("Trajectory reads", || {
                        for item in scratch.iter_mut().filter(|s| s.refresh) {
                            item.motion = Some(trajectories.motion(item.source_index));
                        }
                    });
                    batches.measure("Motion and magnitude calculation", || {
                        for item in scratch.iter_mut().filter(|s| s.refresh) {
                            item.sample = Some(item.motion.unwrap().evaluate(years, item.magnitude));
                        }
                    });
                    batches.measure("Stellar validity qualification", || {
                        for item in scratch.iter_mut().filter(|s| s.refresh) {
                            item.valid_seconds = qualify_stellar_span_counted(
                                item.motion.unwrap(),
                                item.sample.unwrap(),
                                epoch,
                                item.magnitude,
                                maximum,
                                &mut counts,
                            );
                            item.calculated_at = epoch;
                        }
                    });
                    batches.measure("Stellar cache stores", || {
                        for item in scratch.iter().filter(|s| s.refresh) {
                            let entry = self.stellar.get_mut(&item.source_index).expect("cache entry prepared");
                            let before = entry.stats.refreshes;
                            entry.store((), epoch, item.valid_seconds, item.sample.unwrap());
                            self.stellar_stats.refreshes += entry.stats.refreshes - before;
                        }
                    });
                    batches.measure("Motion output assembly", || {
                        for item in &scratch {
                            validity = validity
                                .min((item.valid_seconds - (epoch - item.calculated_at).abs() * 86400.0).max(0.0));
                            let sample = item.sample.unwrap();
                            values.push((sample.direction, sample.magnitude));
                            singular_count += usize::from(sample.used_singular_fallback);
                        }
                    });
                }
            });
            times.describe("Stellar batches", || format!("input/output stars={}; refreshed={refreshed}; reused={reused}; batch limit={BATCH_SIZE}; scratch capacity={} records ({} bytes); refreshed samples with positive validity={}; zero validity: outside interval={}, singular={}, moving with distance={}, zero configured limit={}, boundary/angular bound={}; validity probe evaluations={}; resulting batch validity={validity} s", values.len(), scratch.capacity(), scratch.capacity()*std::mem::size_of::<StellarWork>(), counts.positive, counts.outside_interval, counts.singular, counts.moving_distance, counts.zero_limit, counts.boundary_or_bound, counts.probe_evaluations));
            times.measure("Motion cache store", || {
                self.motion
                    .store(self.working.generation, epoch, validity, (values, singular_count))
            });
            times.measure("Stellar scratch release", || drop(scratch));
        }
        output.runtime_singular_count = self.motion.value().1;
    }
}

/// Keep the same conservative rule and arithmetic as the original fused loop. Reasons are mutually exclusive.
fn qualify_stellar_span_counted(
    motion: StellarMotion,
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
    if motion.distance_pc.is_some() && motion.w != Vector3::default() {
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
            let end = motion.evaluate(years_since_j2000(epoch + offset / 86400.0), magnitude);
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
        let base = ObservedSky::from_catalog(&parsed);
        assert!(base.stars.len() > BATCH_SIZE * 2);
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let mut fused = ObservationCache::new(config.clone());
            let mut batched = ObservationCache::new(config);
            let mut a = base.clone();
            let mut b = base.clone();
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
                batched.update_stellar_motion(&mut b, epoch, &mut times);
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
            qualify_stellar_span_counted(motion, sample, epoch, 4.0, maximum, &mut counts);
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
