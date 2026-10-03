//! Owned geometry caches; borrowed star views are assembled only for the current render call.
use super::{sky::*, *};
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
        times.measure("Star draw order", || {
            let key: Vec<_> = self
                .stars
                .value()
                .iter()
                .map(|&(index, _)| (index, sky.stars[index].magnitude, sky.stars[index].id))
                .collect();
            self.order
                .get_or_update(key, epoch, self.config.allows(Group::DrawOrder), || {
                    let mut order: Vec<_> = (0..self.stars.value().len()).collect();
                    order.sort_unstable_by(|&a, &b| {
                        let a = &sky.stars[self.stars.value()[a].0];
                        let b = &sky.stars[self.stars.value()[b].0];
                        if a.magnitude == b.magnitude {
                            a.id.cmp(&b.id)
                        } else {
                            b.magnitude.total_cmp(&a.magnitude)
                        }
                    });
                    order
                });
        });
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
}
