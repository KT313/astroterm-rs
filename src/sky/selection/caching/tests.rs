//! Regional selection keeps stable versions while the requested set changes.
use super::*;
use crate::{astro::J2000, constants::CONSTELLATION_REGION, model::{SelectedRegion, SkyRegion}, state::StarSelectionCache};
use std::sync::Arc;

fn catalog() -> Arc<SkyCatalog> {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(32);
    source.constellations = vec![crate::catalog::ConstellationFigure {
        abbreviation: "Test", segments: vec![[source.stars[0].hr.unwrap(), source.stars[1].hr.unwrap()]],
    }];
    Arc::new(crate::sky::prepare_catalog(&source).unwrap().catalog)
}

fn storage(catalog: &Arc<SkyCatalog>, config: CacheConfig) -> StarSelectionCache {
    let count = crate::constants::SIMULATION_REGION_COUNT;
    StarSelectionCache { catalog: Some(catalog.clone()), region_candidates: (0..count).map(|_| Default::default()).collect(),
        region_selected: (0..count).map(|_| Default::default()).collect(), ..StarSelectionCache::new(config) }
}

fn populated_regions(catalog: &SkyCatalog) -> [usize; 2] {
    let mut regions = (0..CONSTELLATION_REGION).filter(|&region| catalog.grid.offsets[region] < catalog.grid.offsets[region + 1]);
    [regions.next().unwrap(), regions.next().unwrap()]
}

fn select_cells(storage: &mut StarSelectionCache, catalog: &SkyCatalog, cells: &[usize], threshold: f64, epoch: f64, brute_force: bool) {
    let mut times = StepTimes::default();
    let observer = crate::sky::compose_observer_state(crate::model::FrameTime { utc: epoch, ut1: epoch, tt: epoch }, Default::default(), Default::default(), crate::astro::Matrix3::IDENTITY, Default::default(), false);
    storage.region.store((SkyRegion::All, observer, false), epoch, 0.0, SelectedRegion { cells: cells.to_vec(), brute_force });
    storage.statistics = update_brightness_bounds(&mut storage.region_candidates, &mut storage.candidate_region_stats, &storage.region, &storage.config, epoch, threshold, catalog, &mut times);
    update_candidate_validation(&mut storage.region_selected, &mut storage.selected_region_stats, &storage.region_candidates, &storage.region, &storage.config, epoch, &mut times);
    update_selection_request(&storage.region.value().cells, &storage.region_selected, &mut storage.requested_sources, &mut storage.selection_revision, &mut times);
    update_constellation_endpoints(&mut storage.working, storage.selection_revision, &storage.region.value().cells, &storage.region_selected, storage.statistics.candidates, &storage.config, epoch, catalog.endpoint_indices(), &mut times);
    storage.requested_epoch = Some(epoch);

    let mut expected = Vec::new();
    crate::sky::select_brightness(&catalog.grid, &catalog.stars, storage.region.value(), threshold, &mut expected);
    let actual: Vec<_> = storage.stars().ranges().flat_map(|(_, start, end, _)| start..end).collect(); // test-only materialization for reference comparison
    assert_eq!(actual, expected);
    assert_eq!(storage.statistics.candidates, expected.len());
    let expected_working = crate::sky::merge_constellation_endpoints(expected, catalog.endpoint_indices(), &mut times);
    assert_eq!(storage.working.value(), &expected_working);
}

#[test]
fn overlapping_requests_reuse_individual_regions_and_retain_unrequested_results() {
    let catalog = catalog();
    let [a, b] = populated_regions(&catalog);
    let mut storage = storage(&catalog, CacheConfig::default());
    select_cells(&mut storage, &catalog, &[a, CONSTELLATION_REGION], 20.0, J2000, false);
    let a_version = storage.stars().region_generation(a);
    let endpoint_version = storage.stars().region_generation(CONSTELLATION_REGION);
    assert_eq!(storage.brightness_region_report(b).unwrap().calculated_at, None);
    select_cells(&mut storage, &catalog, &[a, b, CONSTELLATION_REGION], 20.0, J2000 + 1.0, false);
    assert_eq!(storage.stars().region_generation(a), a_version);
    assert_eq!(storage.stars().region_generation(CONSTELLATION_REGION), endpoint_version);
    assert_eq!(storage.validation_region_report(a).unwrap().stats.refreshes, 1);
    let retained_a = storage.validation_region_report(a).unwrap();
    select_cells(&mut storage, &catalog, &[b, CONSTELLATION_REGION], 20.0, J2000 + 2.0, false);
    assert_eq!(storage.validation_region_report(a).unwrap(), retained_a);
    select_cells(&mut storage, &catalog, &[a, CONSTELLATION_REGION], 20.0, J2000 + 3.0, false);
    assert_eq!(storage.stars().region_generation(a), a_version);
    assert_eq!(storage.validation_region_report(a).unwrap().calculated_at, Some(J2000));
    assert_eq!(storage.validation_region_report(a).unwrap().stats.refreshes, 1);
    assert_eq!(storage.brightness_region_report(b).unwrap().stats.refreshes, 1);
}

#[test]
fn threshold_changes_and_explicit_invalidation_preserve_equal_output_versions() {
    let catalog = catalog();
    let [a, b] = populated_regions(&catalog);
    let cells = [a, b, CONSTELLATION_REGION];
    let mut storage = storage(&catalog, CacheConfig::default());
    select_cells(&mut storage, &catalog, &cells, 20.0, J2000, false);
    let generation = storage.stars().region_generation(a);
    select_cells(&mut storage, &catalog, &cells, 21.0, J2000 + 1.0, false);
    assert_eq!(storage.stars().region_generation(a), generation); // a new threshold may select exactly the same prefix
    assert_eq!(storage.validation_region_report(a).unwrap().stats.refreshes, 1);
    let b_report = storage.brightness_region_report(b).unwrap();
    storage.invalidate_region(a);
    select_cells(&mut storage, &catalog, &[b, CONSTELLATION_REGION], 21.0, J2000 + 2.0, false);
    assert!(storage.validation_region_report(a).unwrap().has_been_invalidated);
    select_cells(&mut storage, &catalog, &cells, 21.0, J2000 + 3.0, false);
    assert_eq!(storage.stars().region_generation(a), generation);
    assert_eq!(storage.validation_region_report(a).unwrap().stats.last_reason, Some(crate::cache::RefreshReason::Invalidated));
    assert_eq!(storage.brightness_region_report(b).unwrap().stats.refreshes, b_report.stats.refreshes);
    select_cells(&mut storage, &catalog, &cells, -20.0, J2000 + 4.0, false);
    assert_ne!(storage.stars().region_generation(a), generation);
    assert!(storage.statistics.candidates == 0);
    assert_eq!(storage.working.value().len(), catalog.endpoint_indices().len());
}

#[test]
fn global_and_individual_group_bypasses_refresh_only_requested_region_slots() {
    let catalog = catalog();
    let [a, b] = populated_regions(&catalog);
    for disabled in [None, Some(Group::CandidateSelection), Some(Group::WorkingSet)] {
        let mut config = CacheConfig::default();
        if let Some(group) = disabled { config.groups.insert(group, crate::cache::GroupPolicy { enabled: false, max_age_seconds: None }); }
        else { config.enabled = false; }
        let mut storage = storage(&catalog, config);
        select_cells(&mut storage, &catalog, &[a, CONSTELLATION_REGION], 20.0, J2000, false);
        let generation = storage.stars().region_generation(a);
        select_cells(&mut storage, &catalog, &[a, CONSTELLATION_REGION], 20.0, J2000, false);
        assert_eq!(storage.stars().region_generation(a), generation);
        assert_eq!(storage.brightness_region_report(a).unwrap().stats.bypasses, if disabled != Some(Group::WorkingSet) { 2 } else { 0 });
        assert_eq!(storage.validation_region_report(a).unwrap().stats.bypasses, if disabled != Some(Group::CandidateSelection) { 2 } else { 0 });
        assert_eq!(storage.validation_region_report(b).unwrap().stats.refreshes, 0);
    }
}

#[test]
fn interval_exit_selects_all_stars_and_return_restores_bounded_membership() {
    let catalog = catalog();
    let mut storage = storage(&catalog, CacheConfig::default());
    let cells: Vec<_> = (0..crate::constants::SIMULATION_REGION_COUNT).collect();
    select_cells(&mut storage, &catalog, &cells, -20.0, J2000, false);
    assert!(storage.statistics.candidates == 0);
    select_cells(&mut storage, &catalog, &cells, -20.0, crate::astro::COMPUTATIONAL_INTERVAL.end_tt + 1.0, true);
    assert_eq!(storage.statistics.candidates, catalog.stars.len());
    select_cells(&mut storage, &catalog, &cells, -20.0, J2000, false);
    assert!(storage.statistics.candidates == 0);
}

#[test]
fn replacing_catalog_resets_region_values_versions_and_source_identity() {
    let first = catalog();
    let second = catalog();
    let observer = crate::sky::compose_observer_state(crate::model::FrameTime { utc: J2000, ut1: J2000, tt: J2000 }, Default::default(), Default::default(), crate::astro::Matrix3::IDENTITY, Default::default(), false);
    let mut storage = StarSelectionCache::default();
    let mut times = StepTimes::default();
    crate::sky::select_cached_stars(&mut storage, &first, &observer, 20.0, false, SkyRegion::All, &mut times);
    let first_identity = storage.identity;
    crate::sky::select_cached_stars(&mut storage, &second, &observer, -20.0, false, SkyRegion::All, &mut times);
    assert_ne!(storage.identity, first_identity);
    assert!(Arc::ptr_eq(storage.stars().catalog, &second));
    assert!(storage.statistics.candidates == 0);
    for &region in storage.stars().regions() {
        assert_eq!(storage.validation_region_report(region).unwrap().stats.refreshes, 1);
        assert_eq!(storage.stars().region_generation(region), 1);
    }
}

#[test]
fn paused_slow_and_fast_requests_reuse_ranges_and_request_metadata() {
    let catalog = catalog();
    let [a, b] = populated_regions(&catalog);
    let cells = [a, b, CONSTELLATION_REGION];
    let mut cache = storage(&catalog, CacheConfig::default());
    select_cells(&mut cache, &catalog, &cells, 20.0, J2000, false);
    let revisions = (cache.selection_revision, cache.working.generation);
    let allocation = (cache.requested_sources.as_ptr(), cache.requested_sources.capacity());
    for epoch in [J2000, J2000.next_up(), J2000 + 5000.0, J2000 - 5000.0] {
        let before = (cache.candidate_region_stats.refreshes, cache.selected_region_stats.refreshes, cache.working.stats.refreshes);
        select_cells(&mut cache, &catalog, &cells, 20.0, epoch, false);
        assert_eq!((cache.selection_revision, cache.working.generation), revisions);
        assert_eq!((cache.requested_sources.as_ptr(), cache.requested_sources.capacity()), allocation);
        assert_eq!((cache.candidate_region_stats.refreshes, cache.selected_region_stats.refreshes, cache.working.stats.refreshes), before);
        assert_eq!(cache.stars().ranges().len(), cells.len());
    }
}

#[test]
fn empty_region_changes_request_identity_without_changing_working_rows() {
    let catalog = catalog();
    let empty = (0..CONSTELLATION_REGION).find(|&r| catalog.grid.offsets[r] == catalog.grid.offsets[r + 1]).unwrap();
    let mut cache = storage(&catalog, CacheConfig::default());
    let mut stellar = crate::state::StellarSimulationState::default();
    select_cells(&mut cache, &catalog, &[CONSTELLATION_REGION], -20.0, J2000, false);
    crate::sky::simulate_stars(&mut stellar, cache.stars(), J2000, &mut StepTimes::default());
    let revision = cache.stars().request_revision();
    let working_generation = cache.working.generation;
    let endpoint_generation = stellar.region_report(CONSTELLATION_REGION).unwrap().generation;
    select_cells(&mut cache, &catalog, &[empty, CONSTELLATION_REGION], -20.0, J2000, false);
    assert_ne!(cache.stars().request_revision(), revision);
    assert_eq!(cache.working.generation, working_generation);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stellar.results(cache.stars()))).is_err());
    crate::sky::simulate_stars(&mut stellar, cache.stars(), J2000, &mut StepTimes::default());
    assert_eq!(stellar.results(cache.stars()).selected_count(), catalog.endpoint_indices().len());
    assert_eq!(stellar.region_report(CONSTELLATION_REGION).unwrap().generation, endpoint_generation);
    assert_eq!(stellar.region_report(empty).unwrap().stats.refreshes, 1);
}

#[test]
fn offscreen_threshold_changes_refresh_retained_ranges_on_return() {
    let catalog = catalog();
    let [a, b] = populated_regions(&catalog);
    let mut cache = storage(&catalog, CacheConfig::default());
    select_cells(&mut cache, &catalog, &[a, CONSTELLATION_REGION], 20.0, J2000, false);
    let before = cache.brightness_region_report(a).unwrap();
    select_cells(&mut cache, &catalog, &[b, CONSTELLATION_REGION], -20.0, J2000 + 1.0, false);
    assert_eq!(cache.brightness_region_report(a).unwrap(), before);
    select_cells(&mut cache, &catalog, &[a, CONSTELLATION_REGION], -20.0, J2000 + 2.0, false);
    assert_eq!(cache.statistics.candidates, 0);
    assert_eq!(cache.brightness_region_report(a).unwrap().stats.refreshes, before.stats.refreshes + 1);
    assert!(cache.stars().rows().iter().all(|row| !row.drawable));
    cache.invalidate_view();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cache.stars())).is_err());
}
