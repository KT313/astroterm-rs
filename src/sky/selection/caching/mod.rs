//! Conservative candidate filtering, working indices and drawable membership.
mod diagnostics;
pub(super) use diagnostics::describe_selection;
use crate::state::{RegionCache, WorkingCache};
use crate::model::{SkyCatalog, ObserverState};
use crate::cache::{Cache, CacheConfig, CacheStats, Group};
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain, Access, Operation};
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
    regional: &mut [BoundedPrefixCache], regional_stats: &mut CacheStats, region_cache: &RegionCache,
    config: &CacheConfig, epoch: f64, threshold: f64, catalog: &SkyCatalog, times: &mut StepTimes,
) -> crate::model::SelectionStats {
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
        let count = request.cells.iter().map(|&region| { let (start, end) = regional[region].value(); end - start }).sum();
        crate::model::SelectionStats { cells: request.cells.len() - 1, candidates: count, brute_force: request.brute_force }
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_candidate_validation(
    regional: &mut [Cache<u64, (usize, usize)>], regional_stats: &mut CacheStats,
    bounded: &[BoundedPrefixCache], regions: &RegionCache, config: &CacheConfig, epoch: f64, times: &mut StepTimes,
) {
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
    });
}

pub(super) fn update_selection_request(regions: &[usize], selected: &[Cache<u64, (usize, usize)>], sources: &mut Vec<(usize, u64)>, revision: &mut u64, times: &mut StepTimes) {
    let before = times.inspect_memory(|| BufferShape::vector(sources, IndexDomain::Regions));
    let changed = times.measure("Selection request preparation", || {
        if regions.iter().map(|&id| (id, selected[id].generation)).eq(sources.iter().copied()) { return false; }
        sources.clear();
        sources.extend(regions.iter().map(|&id| (id, selected[id].generation))); // retain metadata capacity across requests
        *revision = revision.checked_add(1).expect("selection request revision exhausted");
        true
    });
    times.record_shape(BufferId::SelectionRequest, if changed { Operation::Build } else { Operation::Reuse }, before,
        || BufferShape::vector(sources, IndexDomain::Regions));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_constellation_endpoints(
    working_cache: &mut WorkingCache, revision: u64, regions: &[usize], selected: &[Cache<u64, (usize, usize)>], candidate_count: usize,
    config: &CacheConfig, epoch: f64, endpoints: &[usize], times: &mut StepTimes,
) {
    times.measure_steps("Constellation endpoints", |times| {
        let refresh = times.measure("Working-set cache decision", || working_cache.needs_refresh(&revision, epoch, None, config.allows(Group::WorkingSet)));
        times.record_candidate_decision(times.last_memory_step(), BufferId::WorkingStars, BufferId::WorkingStars, refresh, working_cache.stats.last_reason);
        if refresh {
            times.record_borrow(BufferId::RegionalValidation, Access::ReadOnly, || BufferShape::slice(selected, IndexDomain::Regions));
            let indices = regions.iter().flat_map(|&region| { let &(start, end) = selected[region].value(); start..end }); // iterate ranges without allocating indices
            let working = merge_sorted_constellation_endpoints(indices, candidate_count, endpoints, times);
            let outcome = times.measure("Working-set cache store", || working_cache.store(revision, epoch, 0.0, working));
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
