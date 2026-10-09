//! Owned geometry caches; borrowed star views are assembled only for the current render call.
mod memory;
mod regions;
mod rendering;
pub use rendering::borrow_render_projection;
pub(super) use regions::project_regional_stars;
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


#[allow(clippy::too_many_arguments)]
pub(super) fn project_cached_stars(storage: &mut ProjectionCache, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) {
    let reuse = storage.config.allows(Group::Projection);
    let (drawable, rejected_projection) = update_star_projection(storage.star_buffers(), sky, view, viewport, epoch, reuse, camera, times);
    times.describe("Star projection", || {
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
pub(super) fn project_cached_bodies(storage: &mut ProjectionCache, sky: crate::model::ObservedSkyView<'_>, view: &View, viewport: Viewport, epoch: f64, camera: CartesianCamera, times: &mut StepTimes) {
    times.measure_with_memory("Body projection", |times| {
        let key = (sky.planets.iter().map(|p| (p.kind, p.position)).collect(), sky.moon, *view, viewport);
        {
            let step = times.active_memory_step();
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedBodies, Access::ReadOnly, BufferShape::unknown(IndexDomain::Objects)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedBodies, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
        }
        update_geometry_cache(&mut storage.bodies, key, epoch, storage.config.allows(Group::Projection), || project_bodies(sky, view, &camera, viewport), times,
            (BufferId::ProjectionBodyCandidate, BufferId::ProjectedBodies));
    });
    times.describe("Body projection", || format!("input Sun/planets={}; visible={}; hidden={}; input Moon=1; visible Moon={}; body list retains hidden records; cache={:?}", sky.planets.len(), storage.bodies.value().0.iter().filter(|p| p.cell.is_some()).count(), storage.bodies.value().0.iter().filter(|p| p.cell.is_none()).count(), usize::from(storage.bodies.value().1.cell.is_some()), storage.bodies.stats));

}

pub(super) fn project_cached_constellations(storage: &mut ProjectionCache, sky: crate::model::ObservedSkyView<'_>, view: &View, viewport: Viewport, epoch: f64, times: &mut StepTimes) {
    times.measure_with_memory("Constellation projection", |times| {
        // Only endpoint geometry affects arcs, not the other stars in the selected region.
        let figures = sky.figures();
        let required = figures.endpoints();
        let endpoints = required.iter().filter_map(|&index| sky.stars.find(index))
            .map(|star| (star.source_index, star.position, star.magnitude)).collect();
        let key = (endpoints, sky.figures().clone(), sky.magnitude_threshold, *view, viewport);
        {
            let step = times.active_memory_step();
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ObservedStars, Access::ReadOnly, BufferShape::unknown(IndexDomain::Observed)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogEndpoints, Access::ReadOnly, BufferShape::slice(required, IndexDomain::Catalog)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::ProjectedFigures, Access::Writable, BufferShape::unknown(IndexDomain::Objects)));
            times.record_memory(step, || MemoryEvent::borrow(BufferId::CatalogFigures, Access::ReadOnly, BufferShape::slice(figures.figures(), IndexDomain::Objects)));
        }
        update_geometry_cache(&mut storage.constellations, key, epoch, storage.config.allows(Group::Projection), || project_constellations(sky.constellations(), sky.stars, sky.magnitude_threshold, view, viewport), times,
            (BufferId::ProjectionFigureCandidate, BufferId::ProjectedFigures));
    });
    times.describe("Constellation projection", || format!("input figures={}; source segments={}; output figures={}; clipped arcs={}; sampled vertices={}; computed regardless of draw toggle; cache={:?}", sky.constellations().len(), sky.constellations().iter().map(|c| c.segments.len()).sum::<usize>(), storage.constellations.value().len(), storage.constellations.value().iter().map(|c| c.arcs.len()).sum::<usize>(), storage.constellations.value().iter().flat_map(|c| &c.arcs).map(|a| a.points.len()).sum::<usize>(), storage.constellations.stats));
    times.describe("Constellation projection", || {
        let missing = sky.constellations().iter().filter(|figure| figure.segments.iter().flatten().any(|index| sky.stars.find(*index).is_none())).count();
        format!("rejected missing endpoints={missing}; then rejected figure magnitude > {}={}; retained figures with no visible arcs={}", sky.magnitude_threshold, sky.constellations().len()-missing-storage.constellations.value().len(), storage.constellations.value().iter().filter(|c| c.arcs.is_empty()).count())
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
pub fn borrow_projected<'a>(storage: &'a ProjectionCache, sky: impl Into<crate::model::ObservedSkyView<'a>>, view: &View, viewport: Viewport) -> ProjectedSky<'a> {
    let sky = sky.into();
    let summary = sky.summary();
    ProjectedSky {
        outside_accuracy_range: sky.outside_accuracy_range,
        selection: sky.selection,
        evaluated_stars: sky.corrections.evaluated,
        correction_stats: sky.corrections,
        catalog_singular_count: sky.catalog.singular_count,
        runtime_singular_count: sky.runtime_singular_count,
        stars: if storage.regional_active {
            crate::model::ProjectedStars::from_regions(sky.stars, &storage.regional_cells, &storage.regional_ranges)
        } else { crate::model::ProjectedStars::with_order(sky.stars, storage.stars.value(), storage.order.value()) },
        planets: &storage.bodies.value().0,
        moon: &storage.bodies.value().1,
        constellations: storage.constellations.value(),
        names: &summary.catalog.names,
        facing: view.is_facing(),
        fov_degrees: view.fov_degrees,
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
fn update_star_projection(buffers: crate::state::StarProjectionBuffers<'_>, sky: &ObservedSky, view: &View, viewport: Viewport, epoch: f64, reuse: bool, camera: CartesianCamera, times: &mut StepTimes) -> (usize, [usize; 2]) {
    let count_drawable = times.trace().is_some();
    let mut drawable = 0;
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
            if count_drawable {
                buffers.candidate.0.extend(sky.stars.iter().map(|s| {
                    drawable += usize::from(s.drawable); // count during the required copy, including cache hits
                    (s.position, s.drawable)
                }));
            } else {
                buffers.candidate.0.extend(sky.stars.iter().map(|s| (s.position, s.drawable)));
            }
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
                let mut cells = Vec::with_capacity(sky.stars.len()); // reserve the maximum before filtering to avoid growth copies
                cells.extend(sky.stars.iter().enumerate().filter(|(_, s)| s.drawable).filter_map(|(index, star)| {
                    let Some(point) = crate::projection::project_camera(camera, star.position) else {
                        rejected[0] += 1;
                        return None;
                    };
                    if !point.is_visible() {
                        rejected[1] += 1;
                        return None;
                    }
                    Some((index, crate::projection::project_to_cell(viewport, point)))
                }));
                cells
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
    (drawable, rejected)
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
        crate::sky::create_sky_from_catalog(&parsed).unwrap()
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
        sky.set_figure_override(Some(crate::sky::prepare_constellation_set(Vec::new(), sky.catalog.stars.len()).unwrap()));
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
                let expected_names: Vec<_> = expected.iter().skip(expected.len().saturating_sub(5)).map(|s| s.id()).collect();
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
                        star.id = StarId(index as u32 + 1);
                        let mixed = (index as u64)
                            .wrapping_mul(0x9e3779b97f4a7c15)
                            .rotate_left(27)
                            .wrapping_mul(0xbf58476d1ce4e5b9);
                        star.magnitude = (mixed % 8192) as f64 / 1024.0 - 3.0; // ties, range -3..5
                        star
                    })
                    .collect();
                sky = crate::sky::create_sky_from_catalog(&crate::catalog::Catalog::new(entries, Default::default(), vec![])).unwrap();
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
    fn shared_topology_matches_reference_and_handles_changed_figures() {
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        let mut cache = ProjectionCache::default();
        let endpoint_storage = sky.catalog.endpoint_indices().as_ptr();
        for phase in 0..6 {
            let mut figures = sky.constellations().to_vec(); // explicit caller-owned replacement, never a runtime copy
            match phase {
                1 => figures.reverse(),
                2 => figures.truncate(1),
                3 => figures.clear(),
                _ => {}
            }
            if phase > 0 { sky.set_figure_override(Some(crate::sky::prepare_constellation_set(figures, sky.catalog.stars.len()).unwrap())); }
            if phase == 4 { sky.set_figure_override(None); }
            let view = View::default();
            let viewport = Viewport { width: 80, height: 40 };
            let previous_refreshes = cache.constellations.stats.refreshes;
            crate::projection::project_cached_sky(&mut cache, &sky, &view, viewport, 0.0, &mut StepTimes::default());
            let actual = crate::projection::borrow_projected(&cache, &sky, &view, viewport);
            let expected_data = crate::projection::project_sky(&sky, &view, viewport);
            assert_eq!(actual, expected_data.view(&sky));
            assert_eq!(sky.catalog.endpoint_indices().as_ptr(), endpoint_storage);
            assert!(std::ptr::eq(actual.names, &sky.catalog.names));
            assert!(std::sync::Arc::ptr_eq(&cache.constellations.key().unwrap().1, sky.figures()) || phase == 5);
            if phase == 5 { assert_eq!(cache.constellations.stats.refreshes, previous_refreshes, "equal replacement sets reuse cached geometry"); }
        }
    }

}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    fn fixture() -> ObservedSky {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars.truncate(6);
        let mut sky = crate::sky::create_sky_from_catalog(&crate::catalog::Catalog::new(parsed.stars, parsed.names, vec![])).unwrap();
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
    fn star_projection_reports_drawable_counts_on_refresh_and_hit() {
        let mut sky = fixture();
        sky.stars[0].drawable = false;
        sky.stars[1].position = Vector3 { x: 0.0, y: 0.0, z: -1.0 };
        sky.stars[2].position = Vector3 { x: 0.75_f64.sqrt(), y: 0.0, z: -0.5 };
        let view = View::default();
        let viewport = Viewport { width: 80, height: 40 };
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let enabled = config.enabled;
            let mut storage = ProjectionCache::new(config);
            for frame in 0..2 {
                let mut times = StepTimes::with_trace(true);
                project_cached_sky(&mut storage, &sky, &view, viewport, 0.0, &mut times);
                let step = times.trace().unwrap().steps.iter().find(|step| step.name == "Star projection").unwrap();
                let details = &step.details[0];
                let rejected = usize::from(frame == 0 || !enabled);
                assert!(details.contains("input observed stars=6; rejected not drawable=1; projection inputs=5;"));
                assert!(details.contains(&format!("rejected singular/invalid={rejected}; then rejected outside unit disk={rejected}; output visible stars=3;")));
                assert_eq!(borrow_projected(&storage, &sky, &view, viewport), crate::projection::project_sky(&sky, &view, viewport).view(&sky));
            }
        }
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
                assert!(std::ptr::eq(entry.star.state.as_ref(), &sky.stars[observed]));
            }
            let reference = crate::projection::project_sky(&sky, &view, viewport);
            assert_eq!(projected, reference.view(&sky));
        }
    }
}
