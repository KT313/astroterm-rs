//! Owned geometry caches; borrowed star views are assembled only for the current render call.
use super::{
    draw_order::{DrawRecord, prepare_draw_order},
    sky::*,
    *,
};
use crate::{
    astro::Vector3,
    cache::{Cache, CacheConfig, Group},
    sky::{ObservedSky, PlanetKind},
    timing::StepTimes,
};

type StarKey = (Vec<(Vector3, bool)>, View, Viewport);
type BodyKey = (Vec<(PlanetKind, Vector3)>, crate::sky::Moon, View, Viewport);
type ConstellationKey = (
    Vec<(usize, Vector3, f64)>,
    Vec<crate::sky::Constellation>,
    f64,
    View,
    Viewport,
);
type HorizonGeometry = (Vec<[Cell; 2]>, Vec<(Cell, &'static str)>);

#[derive(Default)]
pub struct ProjectionCache {
    config: CacheConfig,
    stars: Cache<StarKey, Vec<(usize, Cell)>>,
    order: Cache<Vec<(usize, f64, crate::catalog::StarId)>, Vec<usize>>,
    draw_order_scratch: Vec<DrawRecord>,
    bodies: Cache<BodyKey, (Vec<ProjectedPlanet>, ProjectedMoon)>,
    constellations: Cache<ConstellationKey, Vec<ProjectedConstellation>>,
    horizon: Cache<(View, Viewport), HorizonGeometry>,
}
impl ProjectionCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn invalidate_view(&mut self) {
        self.stars.invalidate();
        self.bodies.invalidate();
        self.constellations.invalidate();
        self.horizon.invalidate();
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.stars.stats,
            self.order.stats,
            self.bodies.stats,
            self.constellations.stats,
            self.horizon.stats,
        ] {
            total.hits += s.hits;
            total.refreshes += s.refreshes;
            total.bypasses += s.bypasses;
        }
        total
    }
    pub fn project<'a>(
        &mut self,
        sky: &'a ObservedSky,
        view: &View,
        viewport: Viewport,
        epoch: f64,
        times: &mut StepTimes,
    ) -> ProjectedSky<'a> {
        let camera = CartesianCamera::new(view);
        times.measure("Star projection", || {
            self.stars.get_or_update(
                (
                    sky.stars.iter().map(|s| (s.position, s.drawable)).collect(),
                    *view,
                    viewport,
                ),
                epoch,
                self.config.allows(Group::Projection),
                || {
                    sky.stars
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| s.drawable)
                        .filter_map(|(index, star)| {
                            project_visible_cell(&camera, viewport, star.position).map(|cell| (index, cell))
                        })
                        .collect()
                },
            );
        });
        times.measure("Star draw order", || self.update_draw_order(sky, epoch));
        times.measure("Body projection", || {
            self.bodies.get_or_update(
                (
                    sky.planets.iter().map(|p| (p.kind, p.position)).collect(),
                    sky.moon.clone(),
                    *view,
                    viewport,
                ),
                epoch,
                self.config.allows(Group::Projection),
                || project_bodies(sky, view, &camera, viewport),
            );
        });
        times.measure("Constellation projection", || {
            // Only endpoint geometry affects arcs, not the other stars in the selected region.
            let mut required: Vec<_> = sky
                .constellations
                .iter()
                .flat_map(|figure| figure.segments.iter().flatten().copied())
                .collect();
            required.sort_unstable();
            required.dedup();
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
            self.constellations.get_or_update(
                (
                    endpoints,
                    sky.constellations.clone(),
                    sky.magnitude_threshold,
                    *view,
                    viewport,
                ),
                epoch,
                self.config.allows(Group::Projection),
                || project_constellations(sky, view, viewport),
            );
        });
        times.measure("Horizon projection", || {
            self.horizon.get_or_update(
                (*view, viewport),
                epoch,
                self.config.allows(Group::ViewGeometry),
                || {
                    (
                        project_horizon_line(view, viewport),
                        project_horizon_labels(view, viewport),
                    )
                },
            );
        });
        ProjectedSky {
            outside_accuracy_range: sky.outside_accuracy_range,
            selection: sky.selection,
            evaluated_stars: sky.corrections.evaluated,
            correction_stats: sky.corrections,
            catalog_singular_count: sky.catalog.singular_count,
            runtime_singular_count: sky.runtime_singular_count,
            stars: self
                .order
                .value()
                .iter()
                .map(|&i| {
                    let (index, cell) = self.stars.value()[i];
                    ProjectedStar {
                        star: &sky.stars[index],
                        cell: Some(cell),
                    }
                })
                .collect(),
            planets: self.bodies.value().0.clone(),
            moon: self.bodies.value().1.clone(),
            constellations: self.constellations.value().clone(),
            names: &sky.names,
            facing: view.is_facing(),
            viewport,
            horizon: self.horizon.value().0.clone(),
            horizon_labels: self.horizon.value().1.clone(),
        }
    }

    fn update_draw_order(&mut self, sky: &ObservedSky, epoch: f64) {
        let key: Vec<_> = self
            .stars
            .value()
            .iter()
            .map(|&(index, _)| (index, sky.stars[index].magnitude, sky.stars[index].id))
            .collect();
        if self
            .order
            .needs_refresh(&key, epoch, None, self.config.allows(Group::DrawOrder))
        {
            prepare_draw_order(
                &mut self.draw_order_scratch,
                key.iter().map(|&(_, magnitude, id)| (magnitude, id)),
            );
            let order = self
                .draw_order_scratch
                .iter()
                .map(|record| record.projected_index)
                .collect();
            self.order.store(key, epoch, 0.0, order);
        }
    }
}

#[cfg(test)]
mod draw_order_tests {
    use super::*;
    use crate::{catalog::StarId, scene::RenderOptions};

    fn create_sky() -> ObservedSky {
        ObservedSky::from_catalog(&crate::catalog::load_embedded_catalog().unwrap())
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
        for (index, star) in sky.stars.iter_mut().enumerate() {
            star.id = StarId(index as u64 + 1);
            star.name = None;
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
                let mut expected: Vec<_> = sky.stars.iter().filter(|s| s.drawable && s.position.z > 0.0).collect();
                expected.sort_unstable_by(|a, b| {
                    if a.magnitude == b.magnitude {
                        a.id.cmp(&b.id)
                    } else {
                        b.magnitude.total_cmp(&a.magnitude)
                    }
                });
                let expected_ids: Vec<_> = expected.iter().map(|s| s.id).collect();
                let expected_names: Vec<_> = expected_ids.iter().rev().take(5).copied().collect();
                for _ in 0..2 {
                    let projected = cache.project(
                        &sky,
                        &View::default(),
                        viewport,
                        phase as f64,
                        &mut StepTimes::default(),
                    );
                    assert_eq!(
                        projected.stars.iter().map(|s| s.star.id).collect::<Vec<_>>(),
                        expected_ids
                    );
                    let names = crate::scene::select_dynamically_named_stars(&options, &projected);
                    assert_eq!(
                        names
                            .into_iter()
                            .map(|i| projected.stars[i].star.id)
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
        let template = sky.stars[0].clone();
        for synthetic_count in [0, 20_000, 250_000] {
            let visible: Vec<_> = if synthetic_count == 0 {
                sky.stars
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.magnitude <= 5.0)
                    .map(|(i, _)| (i, (0, 0)))
                    .collect()
            } else {
                sky.stars = (0..synthetic_count * 2)
                    .map(|index| {
                        let mut star = template.clone();
                        star.id = StarId(index as u64 + 1);
                        let mixed = (index as u64)
                            .wrapping_mul(0x9e3779b97f4a7c15)
                            .rotate_left(27)
                            .wrapping_mul(0xbf58476d1ce4e5b9);
                        star.magnitude = (mixed % 8192) as f64 / 1024.0 - 3.0; // ties, range -3..5
                        star
                    })
                    .collect();
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
                                .map(|&(i, _)| (i, sky.stars[i].magnitude, sky.stars[i].id))
                                .collect();
                            legacy.get_or_update(key, iteration as f64, reuse, || {
                                let mut order: Vec<_> = (0..visible.len()).collect();
                                order.sort_unstable_by(|&a, &b| {
                                    let (a, b) = (&sky.stars[visible[a].0], &sky.stars[visible[b].0]);
                                    if a.magnitude == b.magnitude {
                                        a.id.cmp(&b.id)
                                    } else {
                                        b.magnitude.total_cmp(&a.magnitude)
                                    }
                                });
                                order
                            });
                            black_box(legacy.value());
                        } else {
                            compact.update_draw_order(&sky, iteration as f64);
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
