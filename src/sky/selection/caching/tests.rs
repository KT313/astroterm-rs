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
    update_brightness_bounds(&mut storage.candidates, &mut storage.region_candidates, &mut storage.candidate_region_stats, &storage.region, &storage.config, epoch, threshold, catalog, &mut times);
    update_candidate_validation(&mut storage.selected, &mut storage.region_selected, &mut storage.selected_region_stats, &storage.candidates, &storage.region_candidates, &storage.region, &storage.config, epoch, &mut times);
    update_constellation_endpoints(&mut storage.working, &storage.selected, &storage.config, epoch, catalog.endpoint_indices(), &mut times);
    storage.requested_epoch = Some(epoch);

    let mut expected = Vec::new();
    crate::sky::select_brightness(&catalog.grid, &catalog.stars, storage.region.value(), threshold, &mut expected);
    assert_eq!(&storage.candidates.value().0, &expected);
    assert_eq!(storage.selected.value(), &expected);
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
    assert!(storage.selected.value().is_empty());
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
    assert!(storage.selected.value().is_empty());
    select_cells(&mut storage, &catalog, &cells, -20.0, crate::astro::COMPUTATIONAL_INTERVAL.end_tt + 1.0, true);
    assert_eq!(storage.selected.value().len(), catalog.stars.len());
    select_cells(&mut storage, &catalog, &cells, -20.0, J2000, false);
    assert!(storage.selected.value().is_empty());
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
    assert!(storage.selected.value().is_empty());
    for &region in storage.stars().regions() {
        assert_eq!(storage.validation_region_report(region).unwrap().stats.refreshes, 1);
        assert_eq!(storage.stars().region_generation(region), 1);
    }
}
