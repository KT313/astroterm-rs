//! Cache decisions use region versions. Cells retain catalog indices; orders address membership-versioned region rows.
use std::cmp::Ordering;
use std::sync::Arc;
use crate::cache::{Cache, Group};
use crate::model::{CartesianCamera, ObservedRegion, ProjectionViewport, RegionalDrawRecord, View};
use crate::state::{ProjectionCache, RegionalObservation};
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain, Operation};
use super::memory::record_region_store;

#[allow(clippy::too_many_arguments)]
pub(in crate::projection) fn project_regional_stars(storage: &mut ProjectionCache, observed: RegionalObservation<'_>, view: &View, viewport: ProjectionViewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) {
    times.measure("Regional projection initialization", || prepare_region_storage(storage, observed));                                                // keep unrelated regions until their owner or catalog changes
    let (refreshed, reused, calculated) = times.measure_batches("Star projection", |times| refresh_region_cells(storage, observed, view, viewport, epoch, camera, times));
    let (sorted, reused_order) = times.measure_batches("Star draw order", |times| refresh_region_orders(storage, observed, epoch, times));
    assemble_regional_view(storage, observed, times);                                        // resolve current rows in region order only when regional output changes
    storage.regional_active = true;
    times.describe("Star projection", || format!("regions refreshed={refreshed}; reused={reused}; drawable stars calculated={calculated}; requested regions={}; output visible stars={}; retained regions={}; dependency-only regional keys; catalog indices retained", observed.regions().len(), storage.regional_cells.len(), storage.regional_stars.iter().filter(|region| region.stored().is_some()).count()));
    times.describe("Star draw order", || format!("regions sorted={sorted}; reused={reused_order}; output stars={}; regional magnitude/ID order; ordinary regions drawn independently, constellation region last; dimmest first within each region", storage.regional_cells.len()));
}

fn prepare_region_storage(storage: &mut ProjectionCache, observed: RegionalObservation<'_>) {
    let catalog = observed.sky().catalog;
    let changed = storage.regional_owner != Some(observed.source_id()) || !storage.regional_catalog.as_ref().is_some_and(|old| Arc::ptr_eq(old, catalog));
    if !changed { return; }
    storage.source_revision = storage.source_revision.checked_add(1).expect("projection source revision exhausted");
    storage.regional_stars = (0..catalog.grid.offsets.len() - 1).map(|_| Cache::default()).collect(); // ordinary cells plus the exclusive constellation region
    storage.regional_orders = (0..catalog.grid.offsets.len() - 1).map(|_| Cache::default()).collect();
    storage.regional_catalog = Some(catalog.clone());
    storage.regional_owner = Some(observed.source_id());
    storage.regional_stats = Default::default();
    storage.assembled_for.clear();
    storage.assembly_valid = false;
    storage.region_cell_scratch.clear();
    storage.regional_cell_work.clear();
    storage.regional_order_work.clear();
    storage.regional_ranges.clear();
}

#[allow(clippy::too_many_arguments)]
fn refresh_region_cells(storage: &mut ProjectionCache, observed: RegionalObservation<'_>, view: &View, viewport: ProjectionViewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) -> (usize, usize, usize) {
    let reuse = storage.config.allows(Group::Projection);
    let mut refreshed = 0;
    let mut reused = 0;
    let mut calculated = 0;
    for (slot, region) in observed.regions().iter().enumerate() {
        let key = ((region.selection_generation, region.apparent_generation), observed.horizon_rotation(), observed.refraction_enabled(), *view, viewport);
        let cache = &mut storage.regional_stars[region.region];
        let before = cache.stats;
        let refresh = times.measure("Regional projection decision", || cache.needs_refresh(&key, epoch, None, reuse));
        times.record_candidate_decision(times.last_memory_step(), BufferId::RegionalProjectionKeys, BufferId::RegionalProjectedCells, refresh, cache.stats.last_reason);
        if !refresh { add_stats(&mut storage.regional_stats, before, cache.stats); reused += 1; continue; }
        let sky = observed.sky();
        let stars = sky.stars.region(slot, region);
        let cells = &mut storage.regional_cell_work;
        prepare_region_work(times, "Regional cell work preparation", BufferId::RegionalCellWork, cells, stars.len(), IndexDomain::Catalog);
        times.measure("Regional visible star calculation", || {
            for star in stars.filter(|star| star.drawable) {
                calculated += 1;
                let Some(point) = crate::projection::project_camera(camera, star.position) else { continue; };
                if point.is_visible() { cells.push((star.source_index, crate::projection::project_to_cell(viewport, point))); }
            }
        });
        times.record_build(BufferId::RegionalCellWork, || BufferShape::vector(cells, IndexDomain::Catalog));
        let transition = times.inspect_memory(|| (BufferShape::vector(cells, IndexDomain::Catalog), cache.stored().map(|old| BufferShape::vector(old, IndexDomain::Catalog))));
        let outcome = times.measure("Regional projection store", || cache.store_reusing(key, epoch, 0.0, cells));
        record_region_store(times, BufferId::RegionalCellWork, BufferId::RegionalProjectedCells, transition, cells, IndexDomain::Catalog, outcome);
        add_stats(&mut storage.regional_stats, before, cache.stats);
        refreshed += 1;
    }
    (refreshed, reused, calculated)
}

fn refresh_region_orders(storage: &mut ProjectionCache, observed: RegionalObservation<'_>, epoch: f64, times: &mut StepTimes) -> (usize, usize) {
    let reuse = storage.config.allows(Group::DrawOrder);
    let mut refreshed = 0;
    let mut reused = 0;
    for (slot, region) in observed.regions().iter().enumerate() {
        let key = (region.selection_generation, region.motion_generation);                  // position-only corrections and camera changes do not affect brightness order
        let cache = &mut storage.regional_orders[region.region];
        let before = cache.stats;
        let refresh = times.measure("Regional draw-order decision", || cache.needs_refresh(&key, epoch, None, reuse));
        times.record_candidate_decision(times.last_memory_step(), BufferId::RegionalOrderKeys, BufferId::RegionalDrawOrder, refresh, cache.stats.last_reason);
        if !refresh { add_stats(&mut storage.regional_stats, before, cache.stats); reused += 1; continue; }
        let sky = observed.sky();
        let stars = sky.stars.region(slot, region);
        let order = &mut storage.regional_order_work;
        prepare_region_work(times, "Regional order work preparation", BufferId::RegionalOrderWork, order, stars.len(), IndexDomain::Observed);
        times.measure("Regional sort record construction", || order.extend(stars.enumerate().filter(|(_, star)| star.drawable)
            .map(|(row, star)| (row, star.magnitude, sky.catalog.stars.id(star.source_index)))));
        times.record_build(BufferId::RegionalOrderWork, || BufferShape::vector(order, IndexDomain::Observed));
        times.measure("Regional magnitude and ID sort", || order.sort_unstable_by(compare_records));
        let transition = times.inspect_memory(|| (BufferShape::vector(order, IndexDomain::Observed), cache.stored().map(|old| BufferShape::vector(old, IndexDomain::Observed))));
        let outcome = times.measure("Regional draw-order store", || cache.store_reusing(key, epoch, 0.0, order));
        record_region_store(times, BufferId::RegionalOrderWork, BufferId::RegionalDrawOrder, transition, order, IndexDomain::Observed, outcome);
        add_stats(&mut storage.regional_stats, before, cache.stats);
        refreshed += 1;
    }
    (refreshed, reused)
}

fn prepare_region_work<T>(times: &mut StepTimes, step: &'static str, buffer: BufferId, work: &mut Vec<T>, rows: usize, domain: IndexDomain) {
    let before = times.inspect_memory(|| BufferShape::vector(work, domain));
    times.measure(step, || { work.clear(); work.reserve(rows); });                           // discard partial work on retry; grow only when this region needs more room
    times.with_memory(|times| {
        let after = BufferShape::vector(work, domain);
        times.record_shape(buffer, Operation::Clear, before, || after);
        let operation = if before.is_some_and(|shape| shape.capacity == after.capacity) { Operation::Reuse } else { Operation::Reserve };
        times.record_shape(buffer, operation, before, || after);
    });
}

fn add_stats(total: &mut crate::cache::CacheStats, before: crate::cache::CacheStats, after: crate::cache::CacheStats) {
    total.hits += after.hits - before.hits;
    total.refreshes += after.refreshes - before.refreshes;
    total.bypasses += after.bypasses - before.bypasses;
    total.last_reason = after.last_reason;
}

fn compare_records(a: &RegionalDrawRecord, b: &RegionalDrawRecord) -> Ordering {
    if a.1 == b.1 { a.2.cmp(&b.2) } else { b.1.total_cmp(&a.1) }                              // +0 and -0 share the same brightness, matching the exact fallback
}

fn assembled_region_key(storage: &ProjectionCache, region: &ObservedRegion) -> (usize, usize, usize, u64, u64) {
    (region.region, region.start, region.end, storage.regional_stars[region.region].generation, storage.regional_orders[region.region].generation)
}

fn assemble_regional_view(storage: &mut ProjectionCache, observed: RegionalObservation<'_>, times: &mut StepTimes) {
    let reuse = times.measure("Regional assembly decision", || storage.assembly_valid
        && storage.config.allows(Group::Projection) && storage.config.allows(Group::DrawOrder)
        && observed.regions().iter().map(|region| assembled_region_key(storage, region)).eq(storage.assembled_for.iter().copied()));
    if reuse { return; }                                                                    // compare only small metadata before reusing the drawable cells
    times.measure("Regional projected view assembly", || assemble_visible_cells(storage, observed));
    storage.assembled_for.clear();
    for region in observed.regions() { storage.assembled_for.push(assembled_region_key(storage, region)); }
    storage.assembly_valid = true;
}

fn assemble_visible_cells(storage: &mut ProjectionCache, observed: RegionalObservation<'_>) {
    storage.regional_cells.clear();
    storage.regional_ranges.clear();
    let constellation = crate::constants::CONSTELLATION_REGION;
    let regions = observed.regions().iter().enumerate().filter(|(_, region)| region.region != constellation)
        .chain(observed.regions().iter().enumerate().filter(|(_, region)| region.region == constellation)); // the sky-wide constellation group is always painted last
    for (slot, region) in regions {
        let sky = observed.sky();
        let stars = sky.stars.region(slot, region);
        storage.region_cell_scratch.clear();
        storage.region_cell_scratch.resize(stars.len(), None);                              // reuse a small lookup for just this region
        let mut current = 0;
        for &(source, cell) in storage.regional_stars[region.region].value() {
            while sky.stars.get_regional(slot, region.start + current).source_index < source { current += 1; }
            assert_eq!(sky.stars.get_regional(slot, region.start + current).source_index, source, "regional projection membership must match its observation version");
            storage.region_cell_scratch[current] = Some(cell);
        }
        let start = storage.regional_cells.len();
        for &(row, _, _) in storage.regional_orders[region.region].value() {
            if let Some(cell) = storage.region_cell_scratch[row] { storage.regional_cells.push((crate::model::RegionalStarIndex { region_slot: slot.try_into().expect("region slot fits u32"), observed_index: (region.start + row).try_into().expect("catalog row count fits u32") }, cell)); }
        }
        storage.regional_ranges.push((start, storage.regional_cells.len()));
    }
    storage.region_cell_scratch.clear();                                                   // keep capacity without retaining stale cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{Matrix3, Vector3};
    use crate::cache::CacheConfig;
    use crate::model::{ObservedSky, ProjectionKind};
    use crate::projection::{borrow_projected, project_cached_regions, project_cached_sky, project_sky};

    fn fixture() -> ObservedSky { create_fixture(8) }

    fn create_fixture(count: usize) -> ObservedSky {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars.truncate(count);
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::Catalog::new(parsed.stars, parsed.names, vec![])).unwrap();
        for (index, star) in sky.stars.iter_mut().enumerate() {
            star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            star.drawable = true;
            star.magnitude = [3.0, 1.0, -0.0, 2.0, 0.0, 3.0, 1.0, 0.5][index % 8];
        }
        sky
    }

    fn descriptors() -> [ObservedRegion; 3] {
        [(0, 0, 4), (1, 4, 8), (2, 8, 8)].map(|(region, start, end)| ObservedRegion {
            region, start, end, selection_generation: 1, motion_generation: 1, apparent_generation: 1,
        })
    }

    fn token<'a>(sky: &'a ObservedSky, regions: &'a [ObservedRegion]) -> RegionalObservation<'a> {
        RegionalObservation { sky: sky.into(), regions, owner: Default::default(), horizon: Matrix3::IDENTITY, refraction: false }
    }

    fn check(storage: &mut ProjectionCache, observed: RegionalObservation<'_>, view: &View, viewport: ProjectionViewport) {
        project_cached_regions(storage, observed, view, viewport, 0.0, &mut StepTimes::default());
        assert_regional_result(storage, observed, view, viewport);
    }

    fn assert_regional_result(storage: &ProjectionCache, observed: RegionalObservation<'_>, view: &View, viewport: ProjectionViewport) {
        let actual = borrow_projected(storage, observed.sky(), view, viewport);
        let reference = project_sky(observed.sky(), view, viewport);
        let mut expected = reference.view(observed.sky());
        let constellation = crate::constants::CONSTELLATION_REGION;
        let expected_stars: Vec<_> = observed.regions().iter().filter(|r| r.region != constellation)
            .chain(observed.regions().iter().filter(|r| r.region == constellation)).flat_map(|region| {
                let rows: Vec<_> = (region.start..region.end).map(|index| observed.sky().stars.get(index)).collect();
                expected.stars.iter().filter(move |star| rows.iter().any(|row| row.source_index == star.star.source_index))
            }).collect();
        assert_eq!(actual.stars.iter().collect::<Vec<_>>(), expected_stars);
        expected.stars = actual.stars;                                                      // geometry and metadata still match the independent reference
        assert!(actual == expected, "projected geometry or metadata changed");
    }

    #[test]
    fn constellation_region_draws_last_with_global_labels_selected_independently() {
        let sky = fixture();
        let mut regions = descriptors();
        regions[0].region = crate::constants::CONSTELLATION_REGION;                           // deliberately supply constellation records before ordinary ones
        let observed = token(&sky, &regions);
        let view = View::default(); let viewport = ProjectionViewport { width: 80, height: 40 };
        let options = crate::model::RenderOptions { unicode: true, braille: false, color: true, constellations: false,
            grid: false, dynamic_names: true, magnitude_threshold: 20.0 };
        let mut storage = ProjectionCache::default();
        check(&mut storage, observed, &view, viewport);
        let actual = borrow_projected(&storage, &sky, &view, viewport);
        assert_eq!(actual.stars.iter().map(|s| s.star.source_index).collect::<Vec<_>>(), [5, 6, 7, 4, 0, 3, 1, 2]);
        let reference_data = project_sky(&sky, &view, viewport);
        let reference = reference_data.view(&sky);
        let selected = crate::scene::select_dynamically_named_stars(&options, &actual).map(|i| actual.stars.get(i).star.id()).collect::<Vec<_>>();
        let expected = crate::scene::select_dynamically_named_stars(&options, &reference).map(|i| reference.stars.get(i).star.id()).collect::<Vec<_>>();
        assert_eq!(selected, expected);
        let image = crate::scene::draw_pixel_sky(&actual, &options, &mut StepTimes::default()).unwrap();
        let expected_rgb = crate::scene::star_rgb(&sky.star_view(2));                         // this opaque constellation star is drawn over the ordinary region
        assert_eq!(image.get_pixel(40, 20).0, [expected_rgb[0], expected_rgb[1], expected_rgb[2], 255]);
        let mut scene = crate::state::SceneCache::default();
        assert_eq!(*crate::scene::draw_pixels(&mut scene, &actual, &options, 0.0, &mut StepTimes::default()).unwrap(), image);
    }

    #[test]
    fn regions_reuse_independently_and_resolve_shifted_observed_rows() {
        let mut sky = fixture();
        let original = sky.stars.clone();
        let mut regions = descriptors();
        let owner = token(&sky, &regions).owner;
        let view = View::default();
        let viewport = ProjectionViewport { width: 80, height: 40 };
        let mut storage = ProjectionCache::default();
        for _ in 0..2 { check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport); }
        assert_eq!(storage.regional_stars[0].stats.hits, 1);
        assert_eq!(storage.regional_orders[0].stats.hits, 1);
        assert!(storage.star_candidate.0.is_empty() && storage.order_candidate.is_empty());
        assert!(storage.stars.stored().is_none() && storage.order.stored().is_none());          // trusted path never constructs per-star exact keys

        sky.stars[0].position = Vector3 { x: 1.0, y: 0.0, z: 0.0 };
        regions[0].apparent_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport);
        assert_eq!(storage.regional_stars[0].stats.refreshes, 2);
        assert_eq!(storage.regional_stars[1].stats.refreshes, 1);
        assert_eq!(storage.regional_orders[0].stats.refreshes, 1);                            // position changes do not re-sort a region

        sky.stars = original[4..].to_vec();
        let away = [ObservedRegion { start: 0, end: 4, ..regions[1] }];
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &away) }, &view, viewport);
        assert_eq!(storage.regional_stars[1].stats.refreshes, 1);
        sky.stars = original;
        regions[0].apparent_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport);
        assert_eq!(storage.regional_stars[1].stats.refreshes, 1);                              // returning retains the other region's completed result
        assert_eq!(storage.regional_orders[0].stats.refreshes, 1);

        sky.stars[5].magnitude = -1.0;
        regions[1].motion_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport);
        assert_eq!(storage.regional_orders[1].stats.refreshes, 2);
        assert_eq!(storage.regional_stars[1].stats.refreshes, 1);                             // changing brightness alone does not move a star
        sky.stars[4].drawable = false;
        regions[1].selection_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport);
        assert_eq!(storage.regional_orders[1].stats.refreshes, 3);
        assert_eq!(storage.regional_stars[1].stats.refreshes, 2);
        sky.stars.remove(4);
        regions[1].end -= 1;
        regions[1].selection_generation += 1;
        regions[2].start -= 1;
        regions[2].end -= 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, &view, viewport);
        assert_eq!(storage.regional_orders[1].stats.refreshes, 4);
    }

    #[test]
    fn geometry_dependencies_invalidation_and_fallback_preserve_order() {
        let sky = fixture();
        let regions = descriptors();
        let mut observed = token(&sky, &regions);
        let mut storage = ProjectionCache::default();
        let mut view = View::default();
        let mut viewport = ProjectionViewport { width: 80, height: 40 };
        for phase in 0..7 {
            match phase {
                1 => view.fov_degrees = 90.0,
                2 => viewport.width = 100,
                3 => observed.horizon = Matrix3::rotate_z(0.1),
                4 => observed.refraction = true,
                5 => storage.invalidate_view(),
                6 => view.projection = ProjectionKind::Equidistant,
                _ => {}
            }
            check(&mut storage, observed, &view, viewport);
            assert_eq!(storage.regional_stars[0].stats.refreshes, phase + 1);
            assert_eq!(storage.regional_orders[0].stats.refreshes, 1);
        }
        project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut StepTimes::default());
        assert!(!storage.regional_active);
        assert_eq!(borrow_projected(&storage, &sky, &view, viewport), project_sky(&sky, &view, viewport).view(&sky));
        check(&mut storage, observed, &view, viewport);
        assert_eq!(storage.regional_orders[0].stats.refreshes, 1);
    }

    #[test]
    fn unchanged_assembly_reuses_buffers_but_bypass_rebuilds_cells() {
        let sky = fixture();
        let regions = descriptors();
        let observed = token(&sky, &regions);
        let view = View::default();
        let viewport = ProjectionViewport { width: 80, height: 40 };
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let reuse = config.enabled;
            let mut storage = ProjectionCache::new(config);
            check(&mut storage, observed, &view, viewport);
            let cells = storage.regional_cells.as_ptr();
            let ranges = storage.regional_ranges.as_ptr();
            let mut times = StepTimes::with_trace(true);
            project_cached_regions(&mut storage, observed, &view, viewport, 1.0, &mut times);
            assert!(!times.trace().unwrap().steps.iter().any(|step| step.name == "Regional drawing order merge"));
            let assembled = times.trace().unwrap().steps.iter().any(|step| step.name == "Regional projected view assembly");
            assert_eq!(assembled, !reuse);
            assert_eq!(storage.regional_cells.as_ptr(), cells);
            assert_eq!(storage.regional_ranges.as_ptr(), ranges);
            assert_regional_result(&storage, observed, &view, viewport);
        }
    }

    #[test]
    fn disabled_groups_and_equal_outputs_preserve_regional_generations() {
        let sky = fixture();
        let regions = descriptors();
        let observed = token(&sky, &regions);
        let viewport = ProjectionViewport { width: 80, height: 40 };
        for group in [Group::Projection, Group::DrawOrder] {
            let mut config = CacheConfig::default();
            config.groups.insert(group, crate::cache::GroupPolicy { enabled: false, max_age_seconds: None });
            let mut storage = ProjectionCache::new(config);
            for _ in 0..2 { check(&mut storage, observed, &View::default(), viewport); }
            assert_eq!(storage.regional_stars[0].stats.refreshes, if group == Group::Projection { 2 } else { 1 });
            assert_eq!(storage.regional_orders[0].stats.refreshes, if group == Group::DrawOrder { 2 } else { 1 });
            assert_eq!(storage.regional_stars[0].generation, 1);
            assert_eq!(storage.regional_orders[0].generation, 1);
            let summed_hits: u64 = storage.regional_stars.iter().map(|cache| cache.stats.hits).chain(storage.regional_orders.iter().map(|cache| cache.stats.hits)).sum();
            assert_eq!(storage.regional_stats.hits, summed_hits);
        }
    }

    #[test]
    fn owner_catalog_and_disabled_groups_cannot_reuse_unrelated_results() {
        let sky = fixture();
        let other = fixture();
        let regions = descriptors();
        let view = View::default();
        let viewport = ProjectionViewport { width: 80, height: 40 };
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let reuse = config.enabled;
            let mut storage = ProjectionCache::new(config);
            let observed = token(&sky, &regions);
            for _ in 0..2 { check(&mut storage, observed, &view, viewport); }
            assert_eq!(storage.regional_stars[0].stats.refreshes, if reuse { 1 } else { 2 });
            assert_eq!(storage.regional_orders[0].stats.refreshes, if reuse { 1 } else { 2 });
            check(&mut storage, token(&sky, &regions), &view, viewport);                       // different observation owner, matching generation numbers
            assert_eq!(storage.regional_stars[0].stats.refreshes, 1);
            let observed = RegionalObservation { sky: (&other).into(), ..observed };
            check(&mut storage, observed, &view, viewport);                                  // different catalog allocation with equal rows
            assert!(Arc::ptr_eq(storage.regional_catalog.as_ref().unwrap(), &other.catalog));
        }
    }

    fn allocation_set<T>(entries: &[Cache<impl PartialEq, Vec<T>>], work: &Vec<T>) -> Vec<(usize, usize)> {
        let mut allocations: Vec<_> = entries.iter().filter_map(|entry| entry.stored())
            .chain(std::iter::once(work)).filter(|values| values.capacity() != 0)
            .map(|values| (values.as_ptr() as usize, values.capacity())).collect();
        allocations.sort_unstable();
        allocations
    }

    #[test]
    fn replacement_buffers_stabilize_across_different_region_sizes_and_hits() {
        let sky = fixture();
        let regions = [(0, 0, 1), (1, 1, 8), (2, 8, 8)].map(|(region, start, end)| ObservedRegion {
            region, start, end, selection_generation: 1, motion_generation: 1, apparent_generation: 1,
        });
        let observed = token(&sky, &regions);
        let view = View::default(); let viewport = ProjectionViewport { width: 80, height: 40 };
        let mut storage = ProjectionCache::new(CacheConfig::disabled());
        check(&mut storage, observed, &view, viewport);
        assert!(storage.regional_cell_work.is_empty() && storage.regional_order_work.is_empty());
        assert_eq!(storage.regional_stars[0].value().len(), 1);
        assert_eq!(storage.regional_orders[1].value().len(), 7);
        for _ in 0..4 { check(&mut storage, observed, &view, viewport); }                     // circulate every allocation through the largest region
        let cells = allocation_set(&storage.regional_stars, &storage.regional_cell_work);
        let orders = allocation_set(&storage.regional_orders, &storage.regional_order_work);
        assert!(cells.iter().chain(&orders).all(|&(_, capacity)| capacity >= 7));
        for _ in 0..5 {
            check(&mut storage, observed, &view, viewport);
            assert_eq!(allocation_set(&storage.regional_stars, &storage.regional_cell_work), cells);
            assert_eq!(allocation_set(&storage.regional_orders, &storage.regional_order_work), orders);
            assert!(storage.regional_cell_work.is_empty() && storage.regional_order_work.is_empty());
            assert_eq!(storage.regional_stars[1].generation, 1);
            assert_eq!(storage.regional_orders[1].generation, 1);
        }
        storage.config = CacheConfig::default();
        let work = (storage.regional_cell_work.as_ptr(), storage.regional_order_work.as_ptr());
        let mut times = StepTimes::with_trace(true);
        project_cached_regions(&mut storage, observed, &view, viewport, 1.0, &mut times);
        assert_eq!((storage.regional_cell_work.as_ptr(), storage.regional_order_work.as_ptr()), work);
        assert!(!times.trace().unwrap().steps.iter().any(|step| matches!(step.name,
            "Regional cell work preparation" | "Regional order work preparation" | "Regional magnitude and ID sort")));
    }

    #[test]
    fn larger_membership_grows_work_then_handles_hidden_and_equal_refreshes() {
        let mut sky = create_fixture(256);
        let full_count = sky.stars.len(); // preparation removes any catalog placeholders
        assert!(full_count > 200);
        let mut region = [ObservedRegion { region: 0, start: 0, end: 8, selection_generation: 1, motion_generation: 1, apparent_generation: 1 }];
        let owner = token(&sky, &region).owner;
        let mut storage = ProjectionCache::new(CacheConfig::disabled());
        let viewport = ProjectionViewport { width: 113, height: 71 };
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &region) }, &View::default(), viewport);
        assert_eq!(storage.regional_stars[0].value().len(), 8);
        region[0].end = sky.stars.len(); region[0].selection_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &region) }, &View::default(), viewport);
        assert!(storage.regional_stars[0].value().capacity() >= full_count);
        assert_eq!(storage.regional_stars[0].generation, 2);
        assert_eq!(storage.regional_orders[0].generation, 2);
        for star in &mut sky.stars { star.position.z = -1.0; }
        region[0].apparent_generation += 1;
        for projection in [ProjectionKind::Stereographic, ProjectionKind::Equidistant] {
            let view = View { projection, ..View::default() };
            check(&mut storage, RegionalObservation { owner, ..token(&sky, &region) }, &view, viewport);
            assert!(storage.regional_stars[0].value().is_empty());
            assert_eq!(storage.regional_orders[0].value().len(), full_count);
            assert_eq!(storage.regional_stars[0].generation, 3);
            assert_eq!(storage.regional_orders[0].generation, 2);
        }
        for star in &mut sky.stars { star.drawable = false; }
        region[0].selection_generation += 1;
        check(&mut storage, RegionalObservation { owner, ..token(&sky, &region) }, &View::default(), viewport);
        assert!(storage.regional_orders[0].value().is_empty());
        assert_eq!(storage.regional_orders[0].generation, 3);
    }

    #[test]
    fn owner_reset_clears_partial_work_without_discarding_capacity() {
        let sky = fixture(); let regions = descriptors();
        let observed = token(&sky, &regions);
        let mut storage = ProjectionCache::default();
        check(&mut storage, observed, &View::default(), ProjectionViewport { width: 80, height: 40 });
        storage.regional_cell_work.reserve(32);
        storage.regional_cell_work.push(storage.regional_stars[0].value()[0]);
        storage.regional_order_work.reserve(32);
        storage.regional_order_work.push(storage.regional_orders[0].value()[0]);
        let work = (storage.regional_cell_work.as_ptr(), storage.regional_cell_work.capacity(),
            storage.regional_order_work.as_ptr(), storage.regional_order_work.capacity());
        prepare_region_storage(&mut storage, token(&sky, &regions));                       // new observation owner with matching numerical data
        assert!(storage.regional_stars.iter().all(|entry| entry.stored().is_none()));
        assert!(storage.regional_orders.iter().all(|entry| entry.stored().is_none()));
        assert!(!storage.assembly_valid);
        assert!(storage.regional_cell_work.is_empty() && storage.regional_order_work.is_empty());
        assert_eq!((storage.regional_cell_work.as_ptr(), storage.regional_cell_work.capacity(),
            storage.regional_order_work.as_ptr(), storage.regional_order_work.capacity()), work);
    }

    #[test]
    fn interrupted_order_build_retains_invalid_old_result_and_retries_cleanly() {
        let mut sky = fixture(); let mut regions = descriptors();
        let observed = token(&sky, &regions); let owner = observed.owner;
        let mut storage = ProjectionCache::default(); let mut times = StepTimes::default();
        prepare_region_storage(&mut storage, observed);
        refresh_region_orders(&mut storage, observed, 0.0, &mut times);
        let saved = storage.regional_orders[0].value().clone();
        let saved_pointer = storage.regional_orders[0].value().as_ptr();
        let source = sky.stars[1].source_index;
        sky.stars[1].source_index = usize::MAX;                                             // fail after the first replacement record has been written
        regions[0].selection_generation += 1;
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            refresh_region_orders(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, 1.0, &mut times);
        }));
        assert!(interrupted.is_err());
        assert_eq!(storage.regional_order_work.len(), 1);
        assert!(storage.regional_orders[0].has_been_invalidated);
        assert_eq!(storage.regional_orders[0].stored().unwrap(), &saved);
        assert_eq!(storage.regional_orders[0].stored().unwrap().as_ptr(), saved_pointer);
        assert!(std::panic::catch_unwind(|| storage.regional_orders[0].value()).is_err());
        sky.stars[1].source_index = source;
        refresh_region_orders(&mut storage, RegionalObservation { owner, ..token(&sky, &regions) }, 1.0, &mut times);
        assert_eq!(storage.regional_orders[0].value(), &saved);
        assert_eq!(storage.regional_orders[0].generation, 1);
        assert_eq!(storage.regional_orders[0].calculated_at, Some(1.0));
        assert!(storage.regional_order_work.is_empty());
    }

    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn regional_events_and_inventory_report_growth_reuse_and_retained_work() {
        use crate::timing::MemoryEvent;
        let sky = fixture(); let regions = descriptors(); let observed = token(&sky, &regions);
        let mut storage = ProjectionCache::new(CacheConfig::disabled());
        let mut growth = Vec::new();
        for frame in 0..6 {
            let mut times = StepTimes::with_trace(true); times.enable_memory_events(true);
            project_cached_regions(&mut storage, observed, &View::default(), ProjectionViewport { width: 80, height: 40 }, frame as f64, &mut times);
            let events: Vec<_> = times.trace().unwrap().steps.iter().flat_map(|step| &step.memory_events).map(|record| record.event).collect();
            growth.push(events.iter().any(|event| matches!(event, MemoryEvent::Operation {
                buffer: BufferId::RegionalCellWork | BufferId::RegionalOrderWork, operation: Operation::Reserve, .. })));
            assert!(events.iter().any(|event| matches!(event, MemoryEvent::Operation {
                buffer: BufferId::RegionalProjectedCells, operation: Operation::Move, .. })));
            assert!(events.iter().any(|event| matches!(event, MemoryEvent::Operation {
                buffer: BufferId::RegionalDrawOrder, operation: Operation::Store { value_changed }, .. } if *value_changed == (frame == 0))));
            assert!(!events.iter().any(|event| matches!(event, MemoryEvent::Operation {
                buffer: BufferId::RegionalCellWork | BufferId::RegionalOrderWork | BufferId::RegionalProjectedCells | BufferId::RegionalDrawOrder,
                operation: Operation::Copy, .. })));
        }
        assert!(growth[0]); assert!(!growth[4] && !growth[5]);                                // no capacity growth once all replacement allocations are large enough
        let inventory = crate::state::collect_inventory("projection", &storage);
        for (name, bytes) in [("regional_cell_work", storage.regional_cell_work.capacity() * std::mem::size_of::<(usize, crate::model::Cell)>()),
            ("regional_order_work", storage.regional_order_work.capacity() * std::mem::size_of::<RegionalDrawRecord>())] {
            let row = inventory.rows.iter().find(|row| row.path.ends_with(name) && row.kind == crate::cache::Kind::Heap).unwrap();
            assert_eq!(row.used, Some(0)); assert_eq!(row.reserved, Some(bytes));
        }
    }
}
