//! Conservative candidate filtering, working indices and drawable membership.
mod diagnostics;
pub(super) use diagnostics::describe_selection;
use crate::state::{RegionCache, CandidateCache, SelectedCache, WorkingCache};
use crate::model::{SkyCatalog, ObserverState};
use crate::cache::{Cache, CacheConfig, CacheStats, Group};
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain, Access, MemoryEvent, Operation};
use crate::sky::{snapshot_cache, record_cache};
use super::processing::merge_sorted_constellation_endpoints;

type BoundedPrefixCache = Cache<(f64, bool), (usize, usize)>;

#[allow(clippy::too_many_arguments)]
pub(super) fn update_region_filtering(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn update_brightness_bounds(
    candidates: &mut CandidateCache, regional: &mut [BoundedPrefixCache], regional_stats: &mut CacheStats, region_cache: &RegionCache,
    config: &CacheConfig, epoch: f64, threshold: f64, catalog: &SkyCatalog, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(candidates));
    times.measure_steps("Brightness bounds", |times| {
        let request = region_cache.value();
        let enabled = config.allows(Group::CandidateSelection);
        let previous = *regional_stats;
        times.measure("Regional brightness decisions", || {
            for &region in &request.cells {
                let entry = &mut regional[region];
                let key = (threshold, request.brute_force);
                let before = entry.stats;
                if entry.needs_refresh(&key, epoch, None, enabled) {
                    let start = catalog.grid.offsets[region];
                    let end = catalog.grid.offsets[region + 1];
                    let count = if request.brute_force { end - start } else {
                        catalog.stars.brightness_keys()[start..end].partition_point(|&bound| crate::catalog::passes_brightness_bound(bound, threshold))
                    };
                    entry.store(key, epoch, 0.0, (start, start + count));
                }
                accumulate_regional_stats(regional_stats, before, entry.stats);
            }
        });
        times.record_regional_counts(BufferId::RegionalBrightness, previous, *regional_stats);
        times.describe("Regional brightness decisions", || format!("requested regions={}; hits={}; refreshed={}; bypassed={}; keys contain threshold and interval status; cached output is one catalog prefix per region",
            request.cells.len(), regional_stats.hits - previous.hits, regional_stats.refreshes - previous.refreshes, regional_stats.bypasses - previous.bypasses));
        times.measure("Brightness candidate assembly", || {
            candidates.get_or_update((region_cache.generation, threshold), epoch, config.allows(Group::CandidateSelection), || {
                let count = request.cells.iter().map(|&region| { let (start, end) = regional[region].value(); end - start }).sum();
                let mut indices = Vec::with_capacity(count);
                for &region in &request.cells {
                    let &(start, end) = regional[region].value();
                    indices.extend(start..end);
                }
                let statistics = crate::model::SelectionStats { cells: request.cells.len() - 1, candidates: indices.len(), brute_force: request.brute_force };
                (indices, statistics)
            });
        });
    });
    record_cache(times, BufferId::BrightnessCandidates, memory_before, candidates);
    if memory_before.is_some_and(|(_, stats)| candidates.stats.refreshes != stats.refreshes) {
        times.record_borrow(BufferId::CatalogStars, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_borrow(BufferId::RegionSelection, Access::ReadOnly, || BufferShape::unknown(IndexDomain::Catalog));
        times.record_build(BufferId::BrightnessCandidates, || BufferShape::vector(&candidates.value().0, IndexDomain::Catalog));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_candidate_validation(
    selected_cache: &mut SelectedCache, regional: &mut [Cache<u64, (usize, usize)>], regional_stats: &mut CacheStats, candidates: &CandidateCache,
    bounded: &[BoundedPrefixCache], regions: &RegionCache, config: &CacheConfig, epoch: f64, times: &mut StepTimes,
) {
    let memory_before = times.inspect_memory(|| snapshot_cache(selected_cache));
    times.measure_steps("Candidate validation", |times| {
        let enabled = config.allows(Group::WorkingSet);
        let previous = *regional_stats;
        times.measure("Regional validation decisions", || {
            for &region in &regions.value().cells {
                let input = &bounded[region];
                let before = regional[region].stats;
                regional[region].get_or_update(input.generation, epoch, enabled, || *input.value());
                accumulate_regional_stats(regional_stats, before, regional[region].stats);
            }
        });
        times.record_regional_counts(BufferId::RegionalValidation, previous, *regional_stats);
        times.describe("Regional validation decisions", || format!("requested regions={}; hits={}; refreshed={}; bypassed={}; one upstream regional version check; no per-star cache checks",
            regions.value().cells.len(), regional_stats.hits - previous.hits, regional_stats.refreshes - previous.refreshes, regional_stats.bypasses - previous.bypasses));
        times.measure("Validated candidate assembly", || {
            selected_cache.get_or_update((candidates.generation, crate::astro::COMPUTATIONAL_INTERVAL.contains(epoch)),
                epoch, config.allows(Group::WorkingSet), || {
                    let mut selected = Vec::with_capacity(candidates.value().0.len());
                    for &region in &regions.value().cells {
                        let &(start, end) = regional[region].value();
                        selected.extend(start..end); // the producer already validated this complete catalog prefix
                    }
                    selected
                });
        });
    });
    record_cache(times, BufferId::ValidatedCandidates, memory_before, selected_cache);
    if memory_before.is_some_and(|(_, stats)| selected_cache.stats.refreshes != stats.refreshes) {
        times.record_borrow(BufferId::BrightnessCandidates, Access::ReadOnly, || BufferShape::vector(&candidates.value().0, IndexDomain::Catalog));
        times.record_build(BufferId::ValidatedCandidates, || BufferShape::vector(selected_cache.value(), IndexDomain::Catalog));
    }
}

pub(super) fn update_constellation_endpoints(
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
            let working = merge_sorted_constellation_endpoints(selected_cache.value(), endpoints, times);
            let outcome = times.measure("Working-set cache store", || {
                working_cache.store(selected_cache.generation, epoch, 0.0, working)
            });
            times.record_store(BufferId::WorkingStars, outcome);
        }
    });
}

fn accumulate_regional_stats(total: &mut CacheStats, before: CacheStats, after: CacheStats) {
    total.hits += after.hits - before.hits;
    total.refreshes += after.refreshes - before.refreshes;
    total.bypasses += after.bypasses - before.bypasses;
    if after.refreshes != before.refreshes { total.last_reason = after.last_reason; }
}

#[cfg(test)]
mod tests;
