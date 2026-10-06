//! Owned geometry caches; borrowed star views are assembled only for the current render call.
mod memory;
use memory::{record_key_build, record_cache_store, record_candidate_clear};
use crate::state::ProjectionCache;
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use super::geometry::{prepare_draw_order_with_times, project_bodies, project_constellations, project_horizon_labels, project_horizon_line};
use crate::model::{CartesianCamera, ProjectedSky, ProjectionViewport as Viewport, View};
#[cfg(test)]
use super::pipeline::project_cached_sky;
use crate::{
    cache::Group,
    model::ObservedSky,
    timing::StepTimes,
};


#[cfg(test)]
use crate::astro::Vector3;

# [cfg (test)] use crate::cache::Cache;
# [cfg (test)] use crate::cache::CacheConfig;
# [cfg (test)] use crate::model::DrawRecord;


/// Retain the already prepared endpoint union. Public callers may replace figures; those use a fallback.
pub fn prepare_projection_catalog(storage: &mut ProjectionCache, catalog: &crate::model::SkyCatalog, times: &mut StepTimes) {
    times.measure("Constellation topology", || {
        storage.prepared_figures = catalog.constellations.clone();
        storage.prepared_endpoints = catalog.endpoint_indices.to_vec();
    });
    {
        let step = times.last_memory_step();
        times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogFigures, Access::ReadOnly, BufferShape::slice(&catalog.constellations, IndexDomain::Objects)));
        times.record_memory(step, || MemoryEvent::operation(BufferId::PreparedFigures, Operation::Copy, None, Some(BufferShape::vector(&storage.prepared_figures, IndexDomain::Objects)), Some(storage.prepared_figures.len()), None)); // nested segment allocations are not included
        times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogEndpoints, Access::ReadOnly, BufferShape::slice(&catalog.endpoint_indices, IndexDomain::Catalog)));
        times.record_memory(step, || MemoryEvent::operation(BufferId::PreparedEndpoints, Operation::Copy, None, Some(BufferShape::vector(&storage.prepared_endpoints, IndexDomain::Catalog)), Some(storage.prepared_endpoints.len()), storage.prepared_endpoints.len().checked_mul(std::mem::size_of::<usize>())));
    }
    times.describe("Constellation topology", || {
        format!(
            "figures={}; reused unique sorted endpoints={}; no per-frame endpoint sort for matching figures",
            storage.prepared_figures.len(),
            storage.prepared_endpoints.len()
        )
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_cached_stars(storage: &mut ProjectionCache, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) {
    let reuse = storage.config.allows(Group::Projection);
    let rejected_projection = update_star_projection(storage.star_buffers(), sky, view, viewport, epoch, reuse, camera, times);
    times.describe("Star projection", || {
        let drawable = sky.stars.iter().filter(|s| s.drawable).count();
        format!("input observed stars={}; rejected not drawable={}; projection inputs={drawable}; rejected singular/invalid={}; then rejected outside unit disk={}; output visible stars={}; viewport={}x{}; cache={:?} (rejection counts are newly executed work only)", sky.stars.len(), sky.stars.len()-drawable, rejected_projection[0], rejected_projection[1], storage.stars.value().len(), viewport.width, viewport.height, storage.stars.stats)
    });

}

pub(super) fn project_cached_draw_order(storage: &mut ProjectionCache, sky: &ObservedSky, epoch: f64, times: &mut StepTimes) {
    times.measure_steps("Star draw order", |times| {
        update_draw_order_with_times(storage, sky, epoch, times)
    });
    times.describe("Star draw order", || format!("input/output stars={}; dimmest first, exact f64 magnitude then ascending ID; scratch capacity={}; cache={:?}", storage.order.value().len(), storage.draw_order_scratch.capacity(), storage.order.stats));

}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_cached_bodies(storage: &mut ProjectionCache, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) {
    times.measure_with_memory("Body projection", |times| {
        let key = (sky.planets.iter().map(|p| (p.kind, p.position)).collect(), sky.moon.clone(), *view, viewport);
        {
            let step = times.active_memory_step();
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedBodies, Access::ReadOnly, BufferShape::slice(&sky.planets, IndexDomain::Objects)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedBodies, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
        }
        update_geometry_cache(&mut storage.bodies, key, epoch, storage.config.allows(Group::Projection), || project_bodies(sky, view, &camera, viewport), times,
            (BufferId::ProjectionBodyCandidate, BufferId::ProjectedBodies));
    });
    times.describe("Body projection", || format!("input Sun/planets={}; visible={}; hidden={}; input Moon=1; visible Moon={}; body list retains hidden records; cache={:?}", sky.planets.len(), storage.bodies.value().0.iter().filter(|p| p.cell.is_some()).count(), storage.bodies.value().0.iter().filter(|p| p.cell.is_none()).count(), usize::from(storage.bodies.value().1.cell.is_some()), storage.bodies.stats));

}

pub(super) fn project_cached_constellations(storage: &mut ProjectionCache, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, times: &mut StepTimes) {
    times.measure_with_memory("Constellation projection", |times| {
        // Only endpoint geometry affects arcs, not the other stars in the selected region.
        let mut fallback = Vec::new();
        let prepared = sky.constellations == storage.prepared_figures;
        times.record_memory(times.active_memory_step(), || MemoryEvent::unknown_operation(BufferId::PreparedFigures, Operation::Compare));
        let required = if prepared {
            &storage.prepared_endpoints
        } else {
            fallback.extend(
                sky.constellations
                    .iter()
                    .flat_map(|figure| figure.segments.iter().flatten().copied()),
            );
            fallback.sort_unstable();
            fallback.dedup();
            &fallback
        };
        let endpoints = required
            .iter()
            .filter_map(|index| {
                sky.stars
                    .binary_search_by_key(index, |star| star.source_index)
                    .ok()
                    .map(|i| {
                        let star = &sky.stars[i];
                        (star.source_index, star.position, star.magnitude)
                    })
            })
            .collect();
        let key = (endpoints, sky.constellations.clone(), sky.magnitude_threshold, *view, viewport);
        {
            let step = times.active_memory_step();
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedStars, Access::ReadOnly, BufferShape::slice(&sky.stars, IndexDomain::Observed)));
            times.record_memory(step, || MemoryEvent::borrow(if prepared { BufferId::PreparedEndpoints } else { BufferId::ProjectionFigureCandidate }, Access::ReadOnly, BufferShape::slice(required, IndexDomain::Catalog)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedFigures, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
            times.record_memory(step, || MemoryEvent::operation(BufferId::ProjectionFigureCandidate, Operation::Copy, None, Some(BufferShape::vector(&key.1, IndexDomain::Objects)), Some(key.1.len()), None)); // figures clone nested segment vectors
        }
        update_geometry_cache(&mut storage.constellations, key, epoch, storage.config.allows(Group::Projection), || project_constellations(sky, view, viewport), times,
            (BufferId::ProjectionFigureCandidate, BufferId::ProjectedFigures));
    });
    times.describe("Constellation projection", || format!("input figures={}; source segments={}; output figures={}; clipped arcs={}; sampled vertices={}; computed regardless of draw toggle; cache={:?}", sky.constellations.len(), sky.constellations.iter().map(|c| c.segments.len()).sum::<usize>(), storage.constellations.value().len(), storage.constellations.value().iter().map(|c| c.arcs.len()).sum::<usize>(), storage.constellations.value().iter().flat_map(|c| &c.arcs).map(|a| a.points.len()).sum::<usize>(), storage.constellations.stats));
    times.describe("Constellation projection", || {
        let missing = sky.constellations.iter().filter(|figure| figure.segments.iter().flatten().any(|index| sky.stars.binary_search_by_key(index, |s| s.source_index).is_err())).count();
        format!("rejected missing endpoints={missing}; then rejected figure magnitude > {}={}; retained figures with no visible arcs={}", sky.magnitude_threshold, sky.constellations.len()-missing-storage.constellations.value().len(), storage.constellations.value().iter().filter(|c| c.arcs.is_empty()).count())
    });

}

pub(super) fn project_cached_horizon(storage: &mut ProjectionCache, view: &View, viewport: Viewport, epoch: f64, times: &mut StepTimes) {
    times.measure_with_memory("Horizon projection", |times| {
        times.record_memory(times.active_memory_step(), || MemoryEvent::borrow(BufferId::ProjectedHorizon, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
        update_geometry_cache(&mut storage.horizon, (*view, viewport), epoch, storage.config.allows(Group::ViewGeometry), || (project_horizon_line(view, viewport), project_horizon_labels(view, viewport)), times,
            (BufferId::ProjectionHorizonCandidate, BufferId::ProjectedHorizon));
    });
    times.describe("Horizon projection", || {
        format!(
            "facing={}; output segments={}; labels={}; cache={:?}",
            view.is_facing(),
            storage.horizon.value().0.len(),
            storage.horizon.value().1.len(),
            storage.horizon.stats
        )
    });
}

/// Keep the existing dependency decision, calculation and commit order visible to the current trace step.
fn update_geometry_cache<K: PartialEq, V: PartialEq>(cache: &mut crate::cache::Cache<K, V>, key: K, epoch: f64, reuse: bool, calculate: impl FnOnce() -> V,
    times: &mut StepTimes,
    ids: (BufferId, BufferId),
) {
    let step = times.active_memory_step();
    times.record_memory(step, || MemoryEvent::unknown_operation(ids.0, Operation::Build)); // nested key payload is intentionally not scanned
    let refresh = cache.needs_refresh(&key, epoch, None, reuse);
    times.record_candidate_decision(step, ids.0, ids.1, refresh, cache.stats.last_reason);
    if refresh {
        let value = calculate();
        times.record_memory(step, || MemoryEvent::unknown_operation(ids.1, Operation::Build));
        let outcome = cache.store(key, epoch, 0.0, value);
        {
            times.record_memory(step, || MemoryEvent::unknown_operation(ids.1, Operation::Compare));
            times.record_memory(step, || MemoryEvent::operation(ids.0, Operation::Move, None, None, None, Some(0)));
            times.record_memory(step, || MemoryEvent::unknown_operation(ids.1, Operation::Store { value_changed: outcome.value_changed }));
        }
    } else {
        drop(key); // complete the temporary-key release before recording it
        times.record_memory(step, || MemoryEvent::unknown_operation(ids.0, Operation::Release)); // bounded geometry keys drop on a hit
    }
}

/// Borrow completed geometry; callers must finish consuming it before modifying its backing state.
pub fn borrow_projected<'a>(storage: &'a ProjectionCache, sky: &'a ObservedSky, view: &View, viewport: Viewport) -> ProjectedSky<'a> {
    ProjectedSky {
        outside_accuracy_range: sky.outside_accuracy_range,
        selection: sky.selection,
        evaluated_stars: sky.corrections.evaluated,
        correction_stats: sky.corrections,
        catalog_singular_count: sky.catalog.singular_count,
        runtime_singular_count: sky.runtime_singular_count,
        stars: crate::model::ProjectedStars::new(sky, storage.stars.value(), storage.order.value()),
        planets: &storage.bodies.value().0,
        moon: &storage.bodies.value().1,
        constellations: storage.constellations.value(),
        names: &sky.names,
        facing: view.is_facing(),
        viewport,
        horizon: &storage.horizon.value().0,
        horizon_labels: &storage.horizon.value().1,
    }
}

#[cfg(test)]
fn update_draw_order(storage: &mut ProjectionCache, sky: &ObservedSky, epoch: f64) {
    update_draw_order_with_times(storage, sky, epoch, &mut StepTimes::default());
}

fn update_draw_order_with_times(storage: &mut ProjectionCache, sky: &ObservedSky, epoch: f64, times: &mut StepTimes) {
    let reuse = storage.config.allows(Group::DrawOrder);
    update_order_buffers(storage.order_buffers(), sky, epoch, reuse, times);
}

fn update_order_buffers(buffers: crate::state::DrawOrderBuffers<'_>, sky: &ObservedSky, epoch: f64, reuse: bool, times: &mut StepTimes) {
    {
        let step = times.active_memory_step();
        times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedCells, Access::ReadOnly, BufferShape::slice(buffers.cells, IndexDomain::Visible)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedStars, Access::ReadOnly, BufferShape::slice(&sky.stars, IndexDomain::Observed)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogStars, Access::ReadOnly, BufferShape::unknown(IndexDomain::Catalog))); // stable IDs are read through catalog views
        times.record_memory(step, || MemoryEvent::borrow(BufferId::DrawOrderCandidate, Access::Writable, BufferShape::vector(buffers.candidate, IndexDomain::Visible)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::DrawOrderScratch, Access::Writable, BufferShape::vector(buffers.scratch, IndexDomain::Visible)));
        times.record_memory(step, || MemoryEvent::borrow(BufferId::DrawOrder, Access::Writable, BufferShape::unknown(IndexDomain::DrawOrder)));
    }
    let before_key = times.inspect_memory(|| BufferShape::vector(buffers.candidate, IndexDomain::Visible));
    times.measure("Draw-order cache key", || {
        buffers.candidate.clear();
        buffers.candidate.extend(buffers.cells.iter().map(|&(index, _)| {
            (index, sky.stars[index].magnitude, sky.star_view(index).id())
        }));
    });
    record_key_build(times, BufferId::DrawOrderCandidate, before_key, buffers.candidate, IndexDomain::Visible);
    let key = &*buffers.candidate;
    let refresh = times.measure("Draw-order cache decision", || {
        buffers.order.needs_refresh(key, epoch, None, reuse)
    });
    times.record_candidate_decision(times.last_memory_step(), BufferId::DrawOrderCandidate, BufferId::DrawOrder, refresh, buffers.order.stats.last_reason);
    if refresh {
        prepare_draw_order_with_times(buffers.scratch, key.iter().map(|&(_, magnitude, id)| (magnitude, id)), times);
        let order = times.measure("Draw-order index extraction", || {
            buffers.scratch.iter().map(|record| record.projected_index).collect::<Vec<_>>()
        });
        times.record_memory(times.last_memory_step(), || {
            let shape = BufferShape::vector(&order, IndexDomain::DrawOrder);
            MemoryEvent::operation(BufferId::DrawOrder, Operation::Build, None, Some(shape), shape.len, shape.logical_bytes())
        });
        let before_move = times.inspect_memory(|| BufferShape::vector(buffers.candidate, IndexDomain::Visible));
        let outcome = times.measure("Draw-order cache store", || buffers.order.store(std::mem::take(buffers.candidate), epoch, 0.0, order));
        record_cache_store(times, BufferId::DrawOrderCandidate, BufferId::DrawOrder, before_move, || BufferShape::vector(buffers.candidate, IndexDomain::Visible), outcome);
    } else {
        let before_clear = times.inspect_memory(|| BufferShape::vector(buffers.candidate, IndexDomain::Visible));
        times.measure("Draw-order candidate clear", || buffers.candidate.clear());
        record_candidate_clear(times, BufferId::DrawOrderCandidate, before_clear, buffers.candidate, IndexDomain::Visible);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_star_projection(buffers: crate::state::StarProjectionBuffers<'_>, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, reuse: bool, camera: CartesianCamera, times: &mut StepTimes) -> [usize; 2] {
    let mut rejected = [0; 2];
    times.measure_steps("Star projection", |times| {
        {
            let step = times.active_memory_step();
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedStars, Access::ReadOnly, BufferShape::slice(&sky.stars, IndexDomain::Observed)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectionCandidate, Access::Writable, BufferShape::vector(&buffers.candidate.0, IndexDomain::Observed)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedCells, Access::Writable, BufferShape::unknown(IndexDomain::Visible)));
        }
        let before_key = times.inspect_memory(|| BufferShape::vector(&buffers.candidate.0, IndexDomain::Observed));
        times.measure("Projection cache key", || {
            buffers.candidate.0.clear();
            buffers.candidate.0.extend(sky.stars.iter().map(|s| (s.position, s.drawable)));
            buffers.candidate.1 = *view;
            buffers.candidate.2 = viewport;
        });
        record_key_build(times, BufferId::ProjectionCandidate, before_key, &buffers.candidate.0, IndexDomain::Observed);
        let refresh = times.measure("Projection cache decision", || {
            buffers.cells.needs_refresh(buffers.candidate, epoch, None, reuse)
        });
        times.record_candidate_decision(times.last_memory_step(), BufferId::ProjectionCandidate, BufferId::ProjectedCells, refresh, buffers.cells.stats.last_reason);
        if refresh {
            let cells = times.measure("Visible star calculation", || {
                sky.stars.iter().enumerate().filter(|(_, s)| s.drawable).filter_map(|(index, star)| {
                    let Some(point) = crate::projection::project_camera(camera, star.position) else {
                        rejected[0] += 1;
                        return None;
                    };
                    if !point.is_visible() {
                        rejected[1] += 1;
                        return None;
                    }
                    Some((index, crate::projection::project_to_cell(viewport, point)))
                }).collect::<Vec<_>>()
            });
            times.record_memory(times.last_memory_step(), || {
                let shape = BufferShape::vector(&cells, IndexDomain::Visible);
                MemoryEvent::operation(BufferId::ProjectedCells, Operation::Build, None, Some(shape), shape.len, shape.logical_bytes())
            });
            let before_move = times.inspect_memory(|| BufferShape::vector(&buffers.candidate.0, IndexDomain::Observed));
            let outcome = times.measure("Projection cache store", || buffers.cells.store(std::mem::take(buffers.candidate), epoch, 0.0, cells));
            record_cache_store(times, BufferId::ProjectionCandidate, BufferId::ProjectedCells, before_move, || BufferShape::vector(&buffers.candidate.0, IndexDomain::Observed), outcome);
        } else {
            let before_clear = times.inspect_memory(|| BufferShape::vector(&buffers.candidate.0, IndexDomain::Observed));
            times.measure("Projection candidate clear", || buffers.candidate.0.clear());
            record_candidate_clear(times, BufferId::ProjectionCandidate, before_clear, &buffers.candidate.0, IndexDomain::Observed);
        }
    });
    rejected
}

#[cfg(test)]
mod draw_order_tests {
    use super::*;
    use crate::catalog::StarId;
    use crate::model::RenderOptions;

    fn create_sky() -> ObservedSky {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        for star in &mut parsed.stars {
            star.name = None;
        }
        crate::sky::create_sky_from_catalog(&parsed)
    }

    #[test]
    fn draw_order_refreshes_preserve_membership_ties_and_dynamic_names() {
        let mut sky = create_sky();
        sky.stars.truncate(7);
        for planet in &mut sky.planets {
            planet.position = Vector3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            };
        }
        sky.constellations.clear();
        sky.moon.position = Vector3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        for star in &mut sky.stars {
            star.magnitude = 3.0;
            star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            star.drawable = true;
        }
        let original = sky.stars.clone();
        let options = RenderOptions {
            unicode: true,
            braille: false,
            color: true,
            constellations: false,
            grid: false,
            magnitude_threshold: 5.0,
            label_threshold: -1.0,
            dynamic_names: true,
        };
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let enabled = config.enabled;
            let mut cache = ProjectionCache::new(config);
            let viewport = Viewport { height: 41, width: 81 };
            let mut capacity = 0;
            let mut scratch = std::ptr::null();
            for phase in 0..7 {
                sky.stars.clone_from(&original);
                match phase {
                    1 => {
                        sky.stars[0].magnitude = -0.0;
                        sky.stars[1].magnitude = 0.0;
                    }
                    2 => {
                        sky.stars[0].magnitude = 5.5;
                        sky.stars[0].drawable = false;
                    }
                    3 => sky.stars[2].position.z = -1.0,
                    4 => sky.stars.clear(),
                    5 => sky.stars[6].magnitude = -1.0,
                    _ => {}
                }
                let mut expected: Vec<_> = sky.star_views().filter(|s| s.drawable && s.position.z > 0.0).collect();
                expected.sort_unstable_by(|a, b| {
                    if a.magnitude == b.magnitude {
                        a.id().cmp(&b.id())
                    } else {
                        b.magnitude.total_cmp(&a.magnitude)
                    }
                });
                let expected_ids: Vec<_> = expected.iter().map(|s| s.id()).collect();
                let expected_names: Vec<_> = expected_ids.iter().rev().take(5).copied().collect();
                for _ in 0..2 {
                    crate::projection::project_cached_sky(&mut cache, &sky,
                        &View::default(),
                        viewport,
                        phase as f64,
                        &mut StepTimes::default());
                    let projected = crate::projection::borrow_projected(&cache, &sky, &View::default(), viewport);
                    assert_eq!(
                        projected.stars.iter().map(|s| s.star.id()).collect::<Vec<_>>(),
                        expected_ids
                    );
                    let names = crate::scene::select_dynamically_named_stars(&options, &projected);
                    assert_eq!(
                        names
                            .into_iter()
                            .map(|i| projected.stars.get(i).star.id())
                            .collect::<Vec<_>>(),
                        expected_names
                    );
                    if capacity == 0 {
                        capacity = cache.draw_order_scratch.capacity();
                        scratch = cache.draw_order_scratch.as_ptr();
                    }
                    assert_eq!(cache.draw_order_scratch.capacity(), capacity);
                    assert_eq!(cache.draw_order_scratch.as_ptr(), scratch);
                }
            }
            assert_eq!(cache.order.stats.refreshes, if enabled { 7 } else { 14 });
            assert_eq!(cache.order.stats.hits, if enabled { 7 } else { 0 });
            assert_eq!(cache.order.stats.bypasses, if enabled { 0 } else { 14 });
        }
    }

    #[test]
    #[ignore = "release-only matched draw-order benchmark; BSC t5 and synthetic sort inputs, no AT-HYG"]
    fn compare_compact_draw_order() {
        use std::{hint::black_box, mem::size_of, time::Instant};
        let mut sky = create_sky();
        let template = crate::catalog::load_embedded_catalog().unwrap().stars[0].clone();
        for synthetic_count in [0, 20_000, 250_000] {
            let visible: Vec<_> = if synthetic_count == 0 {
                sky.stars
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.magnitude <= 5.0)
                    .map(|(i, _)| (i, (0, 0)))
                    .collect()
            } else {
                let entries = (0..synthetic_count * 2)
                    .map(|index| {
                        let mut star = template.clone();
                        star.id = StarId(index as u64 + 1);
                        let mixed = (index as u64)
                            .wrapping_mul(0x9e3779b97f4a7c15)
                            .rotate_left(27)
                            .wrapping_mul(0xbf58476d1ce4e5b9);
                        star.magnitude = (mixed % 8192) as f32 / 1024.0 - 3.0; // ties, range -3..5
                        star
                    })
                    .collect();
                sky = crate::sky::create_sky_from_catalog(&crate::catalog::Catalog::new(entries, Default::default(), vec![]));
                sky.stars
                    .sort_unstable_by_key(|star| sky.catalog.stars.id(star.source_index)); // restore synthetic ID order after catalog preparation
                (0..synthetic_count).map(|i| (i * 2, (0, 0))).collect()
            };
            for reuse in [false, true] {
                let mut compact = ProjectionCache::new(if reuse {
                    CacheConfig::default()
                } else {
                    CacheConfig::disabled()
                });
                compact.stars.store(
                    (Vec::new(), View::default(), Viewport { width: 1, height: 1 }),
                    0.0,
                    0.0,
                    visible.clone(),
                );
                let mut legacy = Cache::default();
                let mut durations = [Vec::new(), Vec::new()];
                for iteration in 0..12 {
                    for path in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
                        let start = Instant::now();
                        if path == 0 {
                            let key: Vec<_> = visible
                                .iter()
                                .map(|&(i, _)| (i, sky.stars[i].magnitude, sky.star_view(i).id()))
                                .collect();
                            legacy.get_or_update(key, iteration as f64, reuse, || {
                                let mut order: Vec<_> = (0..visible.len()).collect();
                                order.sort_unstable_by(|&a, &b| {
                                    let (a, b) = (sky.star_view(visible[a].0), sky.star_view(visible[b].0));
                                    if a.magnitude == b.magnitude {
                                        a.id().cmp(&b.id())
                                    } else {
                                        b.magnitude.total_cmp(&a.magnitude)
                                    }
                                });
                                order
                            });
                            black_box(legacy.value());
                        } else {
                            update_draw_order(&mut compact, &sky, iteration as f64);
                            black_box(compact.order.value());
                        }
                        if iteration >= 2 {
                            durations[path].push(start.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    assert_eq!(legacy.value(), compact.order.value());
                }
                for samples in &mut durations {
                    samples.sort_by(f64::total_cmp);
                }
                let median = |samples: &[f64]| (samples[4] + samples[5]) / 2.0;
                println!(
                    "count={} source={} mode={} legacy_ms={:.4} compact_ms={:.4} added_scratch_bytes={} record_bytes={}",
                    visible.len(),
                    if synthetic_count == 0 { "BSC_t5" } else { "synthetic" },
                    if reuse { "hit" } else { "refresh" },
                    median(&durations[0]),
                    median(&durations[1]),
                    compact.draw_order_scratch.capacity() * size_of::<DrawRecord>(),
                    size_of::<DrawRecord>()
                );
            }
        }
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn prepared_topology_matches_reference_and_handles_changed_figures() {
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap());
        let mut cache = ProjectionCache::default();
        let mut startup = StepTimes::with_trace(true);
        crate::projection::prepare_projection_catalog(&mut cache, &sky.catalog, &mut startup);
        let endpoint_storage = cache.prepared_endpoints.as_ptr();
        for phase in 0..4 {
            match phase {
                1 => sky.constellations.reverse(),
                2 => sky.constellations.truncate(1),
                3 => sky.constellations.clear(),
                _ => {}
            }
            let view = View::default();
            let viewport = Viewport { width: 80, height: 40 };
            crate::projection::project_cached_sky(&mut cache, &sky, &view, viewport, 0.0, &mut StepTimes::default());
            let actual = crate::projection::borrow_projected(&cache, &sky, &view, viewport);
            let expected_data = crate::projection::project_sky(&sky, &view, viewport);
            let expected = expected_data.view(&sky);
            assert_eq!(actual, expected);
            assert_eq!(cache.prepared_endpoints.as_ptr(), endpoint_storage);
        }
        assert_eq!(
            startup
                .trace()
                .unwrap()
                .steps
                .iter()
                .filter(|s| s.name == "Constellation topology")
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    fn fixture() -> ObservedSky {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars.truncate(6);
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::Catalog::new(parsed.stars, parsed.names, vec![]));
        for (index, star) in sky.stars.iter_mut().enumerate() {
            star.position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
            star.magnitude = index as f64;
            star.drawable = true;
        }
        sky
    }

    #[test]
    fn candidates_retain_hit_capacity_and_transfer_on_refresh_without_changing_generations() {
        let mut sky = fixture();
        let mut storage = ProjectionCache::default();
        let view = View::default();
        let viewport = Viewport { width: 80, height: 40 };
        let mut times = StepTimes::default();
        project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
        assert_eq!(storage.star_candidate.0.capacity(), 0); // successful refresh transfers the candidate
        assert_eq!(storage.order_candidate.capacity(), 0);
        let generations = (storage.stars.generation, storage.order.generation);

        project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
        let star_buffer = storage.star_candidate.0.as_ptr();
        let order_buffer = storage.order_candidate.as_ptr();
        let capacities = (storage.star_candidate.0.capacity(), storage.order_candidate.capacity());
        assert!(capacities.0 >= sky.stars.len() && capacities.1 >= sky.stars.len());
        for _ in 0..3 {
            project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
            assert!(storage.star_candidate.0.is_empty() && storage.order_candidate.is_empty());
            assert_eq!((storage.star_candidate.0.as_ptr(), storage.order_candidate.as_ptr()), (star_buffer, order_buffer));
            assert_eq!((storage.stars.generation, storage.order.generation), generations);
        }

        storage.invalidate_view();
        project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
        assert_eq!(storage.star_candidate.0.capacity(), 0);
        assert_eq!((storage.stars.generation, storage.order.generation), generations); // equal geometry does not advance generations
        sky.stars[0].magnitude -= 1.0; // changed sort inputs still produce the same order
        project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
        assert_eq!(storage.order_candidate.capacity(), 0);
        assert_eq!(storage.order.generation, generations.1);
    }

    #[test]
    fn projected_views_borrow_geometry_and_observed_rows_across_resizes() {
        let sky = fixture();
        let mut storage = ProjectionCache::default();
        let view = View::default();
        for viewport in [Viewport { width: 40, height: 20 }, Viewport { width: 160, height: 80 }, Viewport { width: 40, height: 20 }] {
            project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut StepTimes::default());
            let projected = borrow_projected(&storage, &sky, &view, viewport);
            assert_eq!(projected.planets.as_ptr(), storage.bodies.value().0.as_ptr());
            assert!(std::ptr::eq(projected.moon, &storage.bodies.value().1));
            assert_eq!(projected.constellations.as_ptr(), storage.constellations.value().as_ptr());
            assert_eq!(projected.horizon.as_ptr(), storage.horizon.value().0.as_ptr());
            for (index, entry) in projected.stars.iter().enumerate() {
                let observed = storage.stars.value()[storage.order.value()[index]].0;
                assert!(std::ptr::eq(entry.star.state, &sky.stars[observed]));
            }
            let reference = crate::projection::project_sky(&sky, &view, viewport);
            assert_eq!(projected, reference.view(&sky));
        }
    }
}
