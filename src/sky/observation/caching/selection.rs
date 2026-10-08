//! Exact brightness eligibility and correction membership over completed stellar results.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::sky::observation) fn update_current_brightness(
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
pub(in crate::sky::observation) fn update_correction_selection(
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
                    output.catalog.endpoint_indices(),
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
            if selection.stats.skipped == 0 {
                let count = selection.indices.len();
                let rows = working[..count].iter().zip(&drawable[..count]).zip(&samples[..count]); // every working row survived, so their order is already correct
                output.stars.extend(rows.map(|((star, &drawable), &(position, magnitude))| ObservedStar {
                    source_index: star.source_index, drawable, position, magnitude,
                }));
            } else {
                output.stars.extend(selection.indices.iter().map(|&index| ObservedStar {
                    source_index: working[index].source_index,
                    drawable: drawable[index],
                    position: samples[index].0,
                    magnitude: samples[index].1,
                }));
            }
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

