//! Small synthetic contracts for complete regional samples, independent ages and exclusive endpoints.
use crate::constants::{CONSTELLATION_REGION, GRID_DEPTH, SIMULATION_REGION_COUNT, STELLAR_REGION_TTL_SECONDS};
use std::sync::Arc;
use crate::{astro::{J2000, Matrix3, Observer, Vector3, models::{BodyState, stars::years_since_j2000}},
    cache::{CacheConfig, Group, GroupPolicy, RefreshReason},
    catalog::{Catalog, ConstellationFigure, SpaceMotion, StarId},
    model::{SkyCatalog, FrameTime, SkyRegion, hash_direction},
    state::{StellarSimulationState, StarSelectionCache}, sky, timing::StepTimes};

fn catalog() -> Arc<SkyCatalog> {
    let template = crate::catalog::load_embedded_catalog().unwrap().stars[0].clone();
    let stars = [(1, 1.5, 4.0), (2, 3.0, 8.0), (3, 0.0, 4.0), (4, 0.0, 9.0), (5, -1.5, 4.0)].map(|(id, ra, mag)| {
        let mut star = template.clone();
        star.id = StarId(id); star.hr = Some(id); star.has_data = true; star.name = None; star.designation = None;
        star.right_ascension = ra; star.declination = 0.0; star.magnitude = mag;
        let direction = crate::astro::Equatorial { right_ascension: ra, declination: 0.0 }.to_unit_vector();
        star.space_motion = Some(SpaceMotion { distance_pc: 1.0, position: direction, velocity: Vector3 { x: 0.0, y: 0.0, z: 0.001 } });
        star
    });
    Arc::new(sky::prepare_owned_catalog(Catalog::new(stars.into(), Default::default(), vec![
        ConstellationFigure { abbreviation: "And", segments: vec![[1, 2], [2, 1], [3, 999]] }
    ])).unwrap().catalog)
}
fn cone(ra: f64) -> SkyRegion { SkyRegion::Cone { center: Vector3 { x: ra.cos(), y: ra.sin(), z: 0.0 }, radius: 0.01 } }
fn observer(tt: f64) -> crate::model::ObserverState {
    let mut observer = sky::compose_observer_state(FrameTime { utc: tt, ut1: tt, tt }, Observer::default(), BodyState::default(), Matrix3::IDENTITY, BodyState::default(), false);
    observer.inertial_to_horizon = Matrix3::IDENTITY;
    observer
}
struct Run { catalog: Arc<SkyCatalog>, selection: StarSelectionCache, stars: StellarSimulationState }
impl Run {
    fn new(config: CacheConfig) -> Self {
        let catalog = catalog();
        let mut stars = StellarSimulationState::new(config.clone());
        sky::prepare_stellar_catalog(&mut stars, catalog.clone(), J2000, &mut StepTimes::default());
        Self { catalog, selection: StarSelectionCache::new(config), stars }
    }
    fn frame(&mut self, tt: f64, region: SkyRegion, threshold: f64) -> StepTimes {
        let mut times = StepTimes::with_trace(true);
        sky::select_cached_stars(&mut self.selection, &self.catalog, &observer(tt), threshold, false, region, &mut times);
        sky::simulate_stars(&mut self.stars, self.selection.stars(), tt, &mut times);
        times
    }
    fn region(&self, id: u32) -> usize {
        let index = self.catalog.stars.iter().position(|s| s.id == StarId(id)).unwrap();
        if self.catalog.endpoint_indices().contains(&index) { CONSTELLATION_REGION }
        else { hash_direction(GRID_DEPTH, self.catalog.stars.stored_direction(index)) }
    }
    fn check_samples_at_stored_epochs(&self) {
        for (id, cache) in self.stars.regions.entries.iter().enumerate() {
            if let Some(samples) = cache.stored() {
                assert_eq!(samples.len(), self.catalog.grid.offsets[id + 1] - self.catalog.grid.offsets[id]);
                for (offset, sample) in samples.iter().enumerate() {
                    let index = self.catalog.grid.offsets[id] + offset;
                    let motion = self.catalog.stars.motion(index);
                    assert_eq!(*sample, motion.evaluate(years_since_j2000(cache.calculated_at.unwrap()), self.catalog.stars.magnitude(index)));
                }
            }
        }
    }
}

#[test]
fn every_star_has_one_region_and_only_actual_unique_endpoints_are_special() {
    let run = Run::new(CacheConfig::default());
    let catalog = &run.catalog;
    assert_eq!(catalog.stars.len(), 5);
    assert_eq!(catalog.grid.offsets.len(), SIMULATION_REGION_COUNT + 1);
    let endpoints: Vec<_> = catalog.endpoint_indices().iter().map(|&i| catalog.stars.id(i)).collect();
    assert_eq!(endpoints, [StarId(1), StarId(2)]);
    assert!(catalog.endpoint_indices().iter().copied().eq(catalog.grid.offsets[CONSTELLATION_REGION]..catalog.stars.len()));
    assert_ne!(run.region(3), CONSTELLATION_REGION); // an unresolved segment must not claim its surviving partner
    assert!(run.stars.regions.entries.iter().all(|c| c.has_been_invalidated && c.calculated_at == Some(J2000) && c.stored().is_none()));
}

#[test]
fn full_regions_include_faint_stars_and_endpoints_do_not_request_geometric_neighbors() {
    let mut run = Run::new(CacheConfig::default());
    let normal = run.region(3);
    let unseen = run.region(5);
    let trace = run.frame(J2000, cone(0.0), 5.0);
    assert_eq!(run.stars.region_samples(normal).unwrap().len(), 2); // magnitude 9 included
    assert_eq!(run.stars.region_samples(CONSTELLATION_REGION).unwrap().len(), 2);
    assert!(run.stars.region_samples(unseen).is_none());
    let endpoint = run.catalog.stars.iter().find(|s| s.id == StarId(1)).unwrap();
    let endpoint_geometric_cell = hash_direction(GRID_DEPTH, endpoint.motion.u0);
    assert!(!run.selection.stars().regions().contains(&endpoint_geometric_cell));
    assert_eq!(run.selection.stars().regions().iter().filter(|&&r| r == CONSTELLATION_REGION).count(), 1);
    let rows: Vec<_> = run.selection.stars().rows().iter().map(|r| (run.catalog.stars.id(r.source_index), r.drawable)).collect();
    assert_eq!(rows, [(StarId(3), true), (StarId(1), true), (StarId(2), false)]);
    assert!(trace.trace().unwrap().steps.len() < 40);
    run.check_samples_at_stored_epochs();

    run.frame(J2000 + 1.0, cone(0.0), 10.0);
    assert_eq!(run.stars.region_report(normal).unwrap().stats.refreshes, 1);
    assert_eq!(run.stars.results(run.selection.stars()).selected_count(), 4);
    run.frame(J2000 + 2.0, cone(0.0), -5.0);
    assert_eq!(run.stars.results(run.selection.stars()).selected_count(), 2); // endpoints still needed, no drawable ordinary stars
    assert_eq!(run.stars.region_report(normal).unwrap().stats.refreshes, 1);
}

#[test]
fn ten_day_boundary_is_absolute_and_non_sliding_in_both_directions() {
    for sign in [-1.0, 1.0] {
        let mut run = Run::new(CacheConfig::default());
        let region = run.region(3);
        run.frame(J2000, cone(0.0), 5.0);
        for days in [0.0, 9.0, -9.0, sign * 10.0] {
            run.frame(J2000 + days, cone(0.0), 5.0);
            let report = run.stars.region_report(region).unwrap();
            assert_eq!(report.calculated_at, Some(J2000));
            assert_eq!(report.valid_seconds, STELLAR_REGION_TTL_SECONDS);
            assert_eq!(report.stats.refreshes, 1);
        }
        let edge = J2000 + sign * 10.0;
        let expired = if sign > 0.0 { edge.next_up() } else { edge.next_down() };
        run.frame(expired, cone(0.0), 5.0);
        let report = run.stars.region_report(region).unwrap();
        assert_eq!(report.calculated_at, Some(expired));
        assert_eq!(report.stats.last_reason, Some(RefreshReason::Expired));
        assert_eq!(report.stats.refreshes, 2);
        run.check_samples_at_stored_epochs();
    }
}

#[test]
fn mixed_ages_invalidation_and_returning_to_a_region_are_independent() {
    let mut run = Run::new(CacheConfig::default());
    let first = run.region(3); let second = run.region(5);
    run.frame(J2000, cone(0.0), 5.0);
    run.frame(J2000 + 5.0, cone(-1.5), 5.0);
    run.frame(J2000 + 11.0, SkyRegion::All, 5.0);
    assert_eq!(run.stars.region_report(first).unwrap().calculated_at, Some(J2000 + 11.0));
    assert_eq!(run.stars.region_report(second).unwrap().calculated_at, Some(J2000 + 5.0));
    assert_eq!(run.stars.region_report(CONSTELLATION_REGION).unwrap().calculated_at, Some(J2000 + 11.0));
    run.stars.invalidate_region(second);
    run.frame(J2000 + 12.0, cone(0.0), 5.0);
    assert!(run.stars.region_report(second).unwrap().has_been_invalidated);
    run.frame(J2000 + 12.0, cone(-1.5), 5.0);
    assert_eq!(run.stars.region_report(second).unwrap().stats.last_reason, Some(RefreshReason::Invalidated));
    run.check_samples_at_stored_epochs();
}

#[test]
fn bypass_and_disabled_group_recalculate_even_paused() {
    let mut group_disabled = CacheConfig::default();
    group_disabled.groups.insert(Group::StellarState, GroupPolicy { enabled: false, max_age_seconds: None });
    for config in [CacheConfig::disabled(), group_disabled] {
        let mut run = Run::new(config);
        let region = run.region(3);
        for _ in 0..2 { run.frame(J2000, cone(0.0), 5.0); }
        assert_eq!(run.stars.region_report(region).unwrap().stats.refreshes, 2);
        assert_eq!(run.stars.region_report(region).unwrap().stats.bypasses, 2);
        run.frame(J2000 + 1.0, cone(0.0), 5.0);
        run.check_samples_at_stored_epochs();
    }
}

#[test]
fn short_override_and_outside_interval_keep_the_explicit_fixed_policy() {
    let mut run = Run::new(CacheConfig::parse("[groups.stellar_state]\nmax_age_seconds=86400").unwrap());
    let region = run.region(3);
    run.frame(J2000, cone(0.0), 5.0);
    run.frame(J2000 + 1.0, cone(0.0), 5.0);
    assert_eq!(run.stars.region_report(region).unwrap().stats.refreshes, 1);
    run.frame((J2000 + 1.0).next_up(), cone(0.0), 5.0);
    assert_eq!(run.stars.region_report(region).unwrap().stats.refreshes, 2);
    let outside = crate::astro::COMPUTATIONAL_INTERVAL.end_tt + 1.0;
    run.frame(outside, cone(0.0), 5.0);
    assert_eq!(run.selection.stars().regions().len(), SIMULATION_REGION_COUNT);
    run.frame(outside + 0.5, cone(0.0), 5.0);
    assert_eq!(run.stars.region_report(region).unwrap().calculated_at, Some(outside));
    run.check_samples_at_stored_epochs();
}

#[test]
fn multiple_numeric_batches_equal_independent_evaluation_and_reuse_scratch() {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.constellations.clear();
    for star in &mut source.stars {
        star.right_ascension = 0.0; star.declination = 0.0; // one populated region spanning many 1024-row batches
    }
    let catalog = Arc::new(sky::prepare_owned_catalog(source).unwrap().catalog);
    let mut selection = StarSelectionCache::default();
    let mut stars = StellarSimulationState::new(CacheConfig::disabled());
    let mut allocation = None;
    for tt in [J2000, J2000 + 1.0, J2000 - 12.0] {
        let mut times = StepTimes::with_trace(true);
        sky::select_cached_stars(&mut selection, &catalog, &observer(tt), 20.0, false, SkyRegion::All, &mut times);
        sky::simulate_stars(&mut stars, selection.stars(), tt, &mut times);
        let actual = stars.results(selection.stars());
        for (index, sample) in actual.selected_samples() {
            let original = catalog.stars.motion(index).evaluate(years_since_j2000(tt), catalog.stars.magnitude(index));
            assert_eq!(*sample, original);
        }
        let current = (stars.stellar_scratch.as_ptr(), stars.stellar_scratch.capacity());
        if let Some(previous) = allocation { assert_eq!(previous, current); }
        allocation = Some(current);
        assert!(stars.stellar_scratch.is_empty());
        assert!(times.trace().unwrap().steps.len() < 45);
    }
}

#[test]
fn all_endpoint_and_empty_catalogs_roundtrip_without_spatial_membership() {
    for count in [0, 2] {
        let mut source = crate::catalog::load_embedded_catalog().unwrap();
        source.stars.retain(|star| star.has_data);
        source.stars.truncate(count);
        for (i, star) in source.stars.iter_mut().enumerate() { star.id = StarId(i as u32 + 1); star.hr = Some(i as u32 + 1); }
        let figures = if count == 0 { vec![] } else { vec![ConstellationFigure { abbreviation: "And", segments: vec![[1, 2], [2, 1]] }] };
        let prepared = sky::prepare_owned_catalog(Catalog::new(source.stars, source.names, figures)).unwrap();
        assert_eq!(prepared.catalog.grid.offsets[CONSTELLATION_REGION], 0);
        assert_eq!(prepared.catalog.grid.offsets[SIMULATION_REGION_COUNT], count);
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("catalog"); let fingerprint = sky::catalog_fingerprint();
        sky::write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
        let loaded = sky::load_cached_catalog(&path, &fingerprint).unwrap();
        assert_eq!(prepared, loaded);
        let catalog = Arc::new(loaded.catalog);
        let mut selection = StarSelectionCache::default(); let mut stars = StellarSimulationState::default(); let mut times = StepTimes::default();
        sky::select_cached_stars(&mut selection, &catalog, &observer(J2000), -10.0, false, cone(0.0), &mut times);
        sky::simulate_stars(&mut stars, selection.stars(), J2000, &mut times);
        assert_eq!(stars.region_samples(CONSTELLATION_REGION).unwrap().len(), count);
        assert_eq!(stars.results(selection.stars()).selected_count(), count);
        assert!(selection.stars().rows().iter().all(|star| !star.drawable));
    }
}

#[test]
fn duplicate_hr_companions_do_not_duplicate_endpoint_membership() {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data); source.stars.truncate(3);
    for (i, star) in source.stars.iter_mut().enumerate() { star.id = StarId(i as u32 + 1); star.hr = Some(if i == 2 { 1 } else { i as u32 + 1 }); }
    let mut parsed = Catalog::new(source.stars, source.names, vec![ConstellationFigure { abbreviation: "And", segments: vec![[1, 2]] }]);
    parsed.hr_representatives.insert(1, StarId(1));
    let prepared = sky::prepare_owned_catalog(parsed).unwrap();
    let catalog = prepared.catalog;
    assert_eq!(catalog.grid.offsets[CONSTELLATION_REGION], 1);
    assert_eq!(catalog.stars.id(0), StarId(3));
    assert_eq!(catalog.endpoint_indices(), &[1, 2]);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn regional_inventory_and_table_count_nested_payload_without_region_row_explosion() {
    use crate::state::Table;
    let mut run = Run::new(CacheConfig::default());
    run.frame(J2000, SkyRegion::All, 5.0);
    let regions = &run.stars.regions;
    let bytes = regions.bytes();
    let expected = regions.entries.len() * std::mem::size_of::<crate::cache::Cache<(), Vec<crate::astro::models::stars::StellarSample>>>()
        + run.catalog.stars.len() * std::mem::size_of::<crate::astro::models::stars::StellarSample>();
    assert_eq!(bytes.used, Some(expected));
    assert_eq!(regions.rows(), SIMULATION_REGION_COUNT);
    assert!(regions.preview().len() <= 20);
    let snapshot = crate::state::collect_inventory("Regions", regions);
    assert!(snapshot.rows.len() < 10);
    assert_eq!(snapshot.omitted_nodes, 0);
    assert_eq!(snapshot.rows.iter().filter(|r| r.kind == crate::cache::Kind::Heap).map(|r| r.used.unwrap()).sum::<usize>(), expected);
}

#[test]
fn zero_stellar_age_reuses_paused_results_but_expires_on_any_time_change() {
    let mut run = Run::new(CacheConfig::parse("[groups.stellar_state]\nmax_age_seconds=0").unwrap());
    let region = run.region(3);
    for _ in 0..2 { run.frame(J2000, cone(0.0), 5.0); }
    assert_eq!(run.stars.region_report(region).unwrap().stats.refreshes, 1);
    assert_eq!(run.stars.region_report(region).unwrap().stats.bypasses, 0);
    run.frame(J2000.next_up(), cone(0.0), 5.0);
    assert_eq!(run.stars.region_report(region).unwrap().stats.refreshes, 2);
    assert_eq!(run.stars.region_report(region).unwrap().stats.last_reason, Some(RefreshReason::Expired));
}

#[test]
fn borrowed_selected_samples_keep_order_and_original_regional_addresses() {
    let mut run = Run::new(CacheConfig::default());
    run.frame(J2000, cone(0.0), 5.0);
    let result = run.stars.results(run.selection.stars());
    let actual: Vec<_> = result.selected_samples().collect();
    assert_eq!(actual.len(), result.selected_count());
    assert!(actual.iter().map(|(index, _)| *index).eq(run.selection.stars().rows().iter().map(|row| row.source_index)));
    for (index, sample) in actual {
        let region = if run.catalog.endpoint_indices().contains(&index) { CONSTELLATION_REGION }
            else { hash_direction(GRID_DEPTH, run.catalog.stars.stored_direction(index)) };
        assert!(std::ptr::eq(sample, &result.region_samples(region)[index - run.catalog.grid.offsets[region]]));
    }
    assert_eq!(result.fallback_count(), result.selected_samples().filter(|(_, sample)| sample.used_singular_fallback).count());
}

#[test]
fn regional_refreshes_reuse_allocations_and_equal_values_keep_versions() {
    let mut run = Run::new(CacheConfig::default());
    let region = run.region(3);
    run.frame(J2000, cone(0.0), 10.0);
    let first = run.stars.region_report(region).unwrap();
    for _ in 0..3 { // populate the one work allocation and let displaced buffers circulate
        run.stars.invalidate_region(region);
        run.stars.invalidate_region(CONSTELLATION_REGION);
        run.frame(J2000, cone(0.0), 10.0);
    }
    let allocations = |state: &StellarSimulationState| {
        let mut buffers: Vec<_> = state.regions.entries.iter().filter_map(|entry| entry.stored())
            .filter(|samples| samples.capacity() > 0).map(|samples| (samples.as_ptr() as usize, samples.capacity())).collect();
        buffers.push((state.region_output_work.as_ptr() as usize, state.region_output_work.capacity()));
        buffers.sort_unstable();
        buffers
    };
    let warmed = allocations(&run.stars);
    for _ in 0..4 {
        run.stars.invalidate_region(region);
        run.stars.invalidate_region(CONSTELLATION_REGION);
        run.frame(J2000, cone(0.0), 10.0);
        assert_eq!(allocations(&run.stars), warmed); // ownership rotates, backing allocations remain alive
        assert_eq!(run.stars.region_report(region).unwrap().generation, first.generation);
        assert!(run.stars.region_output_work.is_empty());
    }
    let hit = run.frame(J2000, cone(0.0), 10.0);
    assert_eq!(allocations(&run.stars), warmed);
    assert!(!hit.trace().unwrap().steps.iter().any(|step| ["Stellar batches", "Selected fallback count", "Motion output assembly"].contains(&step.name)));
    run.frame(J2000 + 11.0, cone(0.0), 10.0);
    assert!(run.stars.region_report(region).unwrap().generation > first.generation);
    run.check_samples_at_stored_epochs();
    assert!(run.stars.regions.entries.iter().filter_map(|entry| entry.stored()).filter(|samples| samples.is_empty()).all(|samples| samples.capacity() == 0));
}

#[test]
fn interrupted_regional_refresh_rejects_publication_and_preserves_previous_samples() {
    let mut run = Run::new(CacheConfig::default());
    let region = run.region(3);
    run.frame(J2000, cone(0.0), 10.0);
    let old = run.stars.region_samples(region).unwrap().to_vec();
    let old_generation = run.stars.region_report(region).unwrap().generation;
    let classes = run.stars.prepared_classes.take().unwrap();
    run.stars.prepared_classes = Some(vec![]); // simulate an interrupted input read before a region can commit
    run.stars.invalidate_region(region);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sky::simulate_stars(&mut run.stars, run.selection.stars(), J2000, &mut StepTimes::default());
    }));
    assert!(failed.is_err());
    assert_eq!(run.stars.regions.entries[region].stored().unwrap(), &old);
    assert_eq!(run.stars.region_report(region).unwrap().generation, old_generation);
    assert!(run.stars.region_samples(region).is_none());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.stars.results(run.selection.stars()))).is_err());
    run.stars.prepared_classes = Some(classes);
    run.frame(J2000, cone(0.0), 10.0);
    assert_eq!(run.stars.region_report(region).unwrap().generation, old_generation);
    assert_eq!(run.stars.results(run.selection.stars()).selected_count(), 4);
}

#[test]
fn fallback_count_covers_selected_samples_and_not_hidden_faint_samples() {
    let mut run = Run::new(CacheConfig::default());
    let normal = run.region(3);
    run.frame(J2000, cone(0.0), 5.0);
    for region in [normal, CONSTELLATION_REGION] {
        let entry = &mut run.stars.regions.entries[region];
        let mut samples = entry.value().clone();
        samples[1].used_singular_fallback = true; // faint ordinary star is excluded, faint endpoint remains required
        let outcome = entry.store((), J2000, STELLAR_REGION_TTL_SECONDS, samples);
        assert!(outcome.value_changed);
        run.stars.region_results_generation += 1;
    }
    run.frame(J2000, cone(0.0), 5.0);
    assert_eq!(run.stars.results(run.selection.stars()).fallback_count(), 1);
    run.frame(J2000, cone(0.0), 10.0);
    assert_eq!(run.stars.results(run.selection.stars()).fallback_count(), 2);
    run.frame(J2000, cone(0.0), -10.0);
    assert_eq!(run.stars.results(run.selection.stars()).fallback_count(), 1);
}

#[test]
fn selection_at_another_time_requires_a_completed_request_even_with_equal_rows() {
    let mut run = Run::new(CacheConfig::default());
    run.frame(J2000, cone(0.0), 5.0);
    let mut times = StepTimes::default();
    sky::select_cached_stars(&mut run.selection, &run.catalog, &observer(J2000 + 1.0), 5.0, false, cone(0.0), &mut times);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.stars.results(run.selection.stars()))).is_err());
    sky::simulate_stars(&mut run.stars, run.selection.stars(), J2000 + 1.0, &mut times);
    assert_eq!(run.stars.results(run.selection.stars()).selected_count(), 3);
    assert_eq!(run.stars.region_report(run.region(3)).unwrap().calculated_at, Some(J2000));
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn work_reservations_stop_growing_after_warmup_and_hits_do_not_touch_work() {
    use crate::timing::{BufferId, MemoryEvent, Operation};
    let mut run = Run::new(CacheConfig::default());
    let normal = run.region(3);
    let mut growth = Vec::new();
    for _ in 0..5 {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_events(true);
        sky::select_cached_stars(&mut run.selection, &run.catalog, &observer(J2000), 10.0, false, cone(0.0), &mut times);
        if !run.stars.regions.entries.is_empty() {
            run.stars.invalidate_region(normal);
            run.stars.invalidate_region(CONSTELLATION_REGION);
        }
        sky::simulate_stars(&mut run.stars, run.selection.stars(), J2000, &mut times);
        let mut grew = false;
        for event in times.trace().unwrap().steps.iter().flat_map(|step| &step.memory_events) {
            if let MemoryEvent::Operation { buffer: BufferId::StellarOutputWork, operation: Operation::Reserve, before: Some(before), after: Some(after), .. } = event.event {
                grew |= after.capacity > before.capacity;
            }
        }
        growth.push(grew);
    }
    assert!(growth[0]); // initial regional population allocates
    assert!(!growth[3] && !growth[4]); // stabilized refreshes reserve within retained capacity
    let mut times = StepTimes::with_trace(true);
    times.enable_memory_events(true);
    sky::simulate_stars(&mut run.stars, run.selection.stars(), J2000, &mut times);
    assert!(!times.trace().unwrap().steps.iter().flat_map(|step| &step.memory_events).any(|record| matches!(record.event,
        MemoryEvent::Operation { buffer: BufferId::StellarOutputWork, .. })));
}
