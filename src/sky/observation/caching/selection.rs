//! Conservative candidate filtering, working indices and drawable membership.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::sky::observation) fn update_region_filtering(
    region_cache: &mut RegionCache, config: &CacheConfig, epoch: f64, refraction: bool,
    region: crate::model::SkyRegion, observer: &ObserverState, grid: &crate::model::SkyGrid, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(region_cache));
    times.measure("Region filtering", || {
        region_cache.get_or_update(
            (region, *observer, refraction),
            epoch,
            config.allows(Group::CandidateSelection),
            || crate::sky::select_region(grid, region, observer, refraction && observer.atmosphere),
        );
    });
    record_cache(times, BufferId::RegionSelection, memory_before, region_cache);
    times.record_borrow(BufferId::CatalogGrid, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
}

pub(in crate::sky::observation) fn update_brightness_bounds(
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
                let stats = crate::sky::select_brightness(&catalog.grid, &catalog.stars,
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

pub(in crate::sky::observation) fn update_candidate_validation(
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

pub(in crate::sky::observation) fn update_constellation_endpoints(
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

