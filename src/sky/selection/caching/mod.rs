//! Conservative candidate filtering, working indices and drawable membership.
mod diagnostics;
pub(super) use diagnostics::describe_selection;
use crate::state::{RegionCache, CandidateCache, SelectedCache, WorkingCache};
use crate::model::{SkyCatalog, ObserverState};
use crate::cache::{CacheConfig, Group};
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain, Access, MemoryEvent, Operation};
use crate::sky::{snapshot_cache, record_cache};
use super::processing::{filter_brightness_candidates, merge_constellation_endpoints};

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

pub(super) fn update_brightness_bounds(
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

pub(super) fn update_candidate_validation(
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
