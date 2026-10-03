//! Four-stage pipeline: immutable catalog and independently cached geometric simulation -> observer-relative sky
//! -> camera projection -> rendering. Astronomy families live in astro::models; no camera enters observation.
//! Common states use f64 J2000 equatorial AU/AU-day, with a barycentric origin. Earth is the only real
//! anchor, with a WGS84 sea-level site. Observation applies light-time, exact parallax and aberration before
//! horizon rotation and optional refraction; camera projection never changes these values.

pub mod cache;
pub mod grid;
mod illumination;
mod objects;
mod storage;
pub use grid::{SelectionStats, SkyRegion};
pub use illumination::{MoonIllumination, compute_moon_illumination};
pub use storage::StarStorage;
mod observation;
mod positions;
pub mod simulation;
pub use observation::{
    Anchor, ObserverState, observe_sky, observe_sky_candidates, prepare_light_time_samples, prepare_observation,
    prepare_observer,
};
pub use simulation::{
    FrameTime, ModelFamily, RefreshCounts, SimulationError, SimulationState, StateRequest, update_simulation,
};

pub use objects::{Constellation, Moon, ObservedStar, Planet, PlanetKind, Star, create_moon, create_planets};
pub use positions::{refract_sky_positions, update_sky_positions};

use std::collections::HashMap;
use std::sync::Arc;

use crate::catalog::{Catalog, ConstellationFigure, StarNames};

/// Immutable model inputs and display metadata, shared by all observed skies.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyCatalog {
    /// Compact immutable inputs, sorted by cell then conservative brightness key and descending stable ID.
    pub stars: StarStorage,
    pub grid: grid::SkyGrid,
    /// Sorted union of all constellation endpoints, independent of candidate selection.
    pub endpoint_indices: crate::catalog::cache::CatalogArray<usize>,
    /// Indices whose trajectory drift exceeds the grid-margin threshold.
    pub always_checked: crate::catalog::cache::CatalogArray<usize>,
    pub singular_count: usize,
    pub names: StarNames,
    pub constellations: Vec<Constellation>,
}

impl SkyCatalog {
    /// Prepare a sorted immutable catalog and resolve constellation endpoints.
    pub fn from_catalog(catalog: &Catalog) -> SkyCatalog {
        Self::build(
            catalog.stars.iter().cloned(),
            catalog.names.clone(),
            &catalog.hr_representatives,
            &catalog.constellations,
        )
    }

    /// Consume parsed rows while compacting them; no second expanded star table is built.
    pub fn from_owned_catalog(catalog: Catalog) -> Self {
        Self::build(
            catalog.stars.into_iter(),
            catalog.names,
            &catalog.hr_representatives,
            &catalog.constellations,
        )
    }

    fn build(
        entries: impl Iterator<Item = crate::catalog::CatalogStar>,
        names: StarNames,
        representatives: &HashMap<u32, crate::catalog::StarId>,
        figures: &[ConstellationFigure],
    ) -> Self {
        // prepare directly into compact arrays, then permute in place by cell, key and stable ID
        let mut stars = StarStorage::default();
        stars.reserve(entries.size_hint().1.unwrap_or(0));
        for entry in entries.filter(|s| s.has_data) {
            stars.push(Star::from_catalog_star(&entry));
        }
        stars.shrink_to_fit();
        let cells: Vec<_> = (0..stars.len()).map(|i| grid::stored_cell(&stars, i)).collect();
        let mut order: Vec<_> = (0..stars.len()).collect();
        order.sort_unstable_by(|&a, &b| {
            cells[a]
                .cmp(&cells[b])
                .then_with(|| stars.brightness_key(a).total_cmp(&stars.brightness_key(b)))
                .then_with(|| stars.id(b).cmp(&stars.id(a)))
        });
        stars.reorder(&order);
        drop(order);
        drop(cells);
        let grid = grid::SkyGrid::build(&stars);

        // resolve representatives after sorting, without changing the choice made before overrides
        let hr_by_id: HashMap<_, _> = representatives.iter().map(|(&hr, &id)| (id, hr)).collect();
        let index_by_hr: HashMap<u32, usize> = stars
            .iter()
            .enumerate()
            .filter_map(|(index, star)| Some((*hr_by_id.get(&star.id)?, index)))
            .collect();
        let constellations: Vec<Constellation> = figures
            .iter()
            .filter_map(|figure| resolve_constellation_figure(figure, &index_by_hr))
            .collect();

        let mut endpoint_indices: Vec<_> = constellations
            .iter()
            .flat_map(|figure| figure.segments.iter().flatten())
            .copied()
            .collect();
        endpoint_indices.sort_unstable();
        endpoint_indices.dedup();
        let always_checked: Vec<_> = stars
            .iter()
            .enumerate()
            .filter_map(|(i, star)| {
                (star.motion_bound > crate::astro::models::stars::ALWAYS_CHECKED_ANGLE).then_some(i)
            })
            .collect();
        let singular_count = stars.iter().filter(|star| star.singular_fallback).count();
        SkyCatalog {
            endpoint_indices: endpoint_indices.into(),
            always_checked: always_checked.into(),
            singular_count,
            stars,
            grid,
            names,
            constellations,
        }
    }

    /// Number of possible-brightness candidates across all cells for an inclusive threshold.
    pub fn count_bright_stars(&self, threshold: f64) -> usize {
        (0..self.stars.len())
            .filter(|&i| self.stars.brightness_key(i) <= threshold)
            .count()
    }

    /// Resolve a star's name from the sky-owned string block.
    pub fn star_name(&self, star: &Star) -> Option<&str> {
        self.names.get(star.name)
    }
}

/// Read-only output of observation, independent of camera projection. It may cover only the requested SkyRegion;
/// request All when reusing one observation for arbitrary cameras. Catalog data is shared across sites and frames.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedSky {
    pub magnitude_threshold: f64,
    pub selection: SelectionStats,
    pub(crate) candidate_indices: Vec<usize>,
    pub runtime_singular_count: usize,
    pub catalog: Arc<SkyCatalog>,
    pub stars: Vec<ObservedStar>,
    pub planets: Vec<Planet>,
    pub moon: Moon,
    pub names: crate::catalog::StarNames,
    pub constellations: Vec<Constellation>,
    pub(crate) refracted: bool,
    pub outside_accuracy_range: bool,
}
/// Compatibility name for the observed sky; simulation caches are a separate type.
pub type Sky = ObservedSky;

impl ObservedSky {
    pub fn new(catalog: Arc<SkyCatalog>) -> Self {
        Self {
            magnitude_threshold: f64::INFINITY,
            selection: SelectionStats::default(),
            candidate_indices: Vec::new(),
            runtime_singular_count: 0,
            names: catalog.names.clone(),
            constellations: catalog.constellations.clone(),
            catalog,
            stars: Vec::new(),
            planets: create_planets(),
            moon: create_moon(),
            refracted: false,
            outside_accuracy_range: false,
        }
    }
    /// Build a fixture with zero positions. Runtime callers should construct SkyCatalog once and call observe_sky.
    pub fn from_catalog(catalog: &Catalog) -> Self {
        let catalog = Arc::new(SkyCatalog::from_catalog(catalog));
        let mut sky = Self::new(catalog);
        sky.stars = sky
            .catalog
            .stars
            .iter()
            .enumerate()
            .map(|(index, star)| {
                ObservedStar::from_star(&star, index, crate::astro::Horizontal::default().to_unit_vector())
            })
            .collect();
        sky
    }
    pub fn count_bright_stars(&self, threshold: f64) -> usize {
        self.stars
            .iter()
            .filter(|star| star.drawable && star.magnitude <= threshold)
            .count()
    }
    pub fn star_name(&self, star: &ObservedStar) -> Option<&str> {
        self.names.get(star.name)
    }
    pub fn sun(&self) -> &Planet {
        &self.planets[0]
    }
}

/// A figure with its stars as indices into the star table. Segments with a star missing from the dataset are left
/// out, and so is a figure without any segments left.
fn resolve_constellation_figure(
    figure: &ConstellationFigure,
    index_by_hr: &HashMap<u32, usize>,
) -> Option<Constellation> {
    let segments: Vec<[usize; 2]> = figure
        .segments
        .iter()
        .filter_map(|[a, b]| Some([*index_by_hr.get(a)?, *index_by_hr.get(b)?]))
        .collect();
    (!segments.is_empty()).then_some(Constellation {
        abbreviation: figure.abbreviation,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogStar, ConstellationFigure, Designation, load_embedded_catalog};

    fn build_sky() -> Sky {
        Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"))
    }

    #[test]
    fn ids_survive_compaction_and_brightness_sorting() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert_eq!(
            sky.star_name(sky.stars.iter().find(|star| star.id.0 == 7001).unwrap()),
            Some("Vega")
        );
        assert!(
            sky.stars
                .iter()
                .all(|star| star.designation == Some(Designation::Hr(star.id.0 as u32)))
        );
        for range in sky.catalog.grid.offsets.windows(2) {
            let stars = &sky.catalog.stars;
            for i in range[0] + 1..range[1] {
                assert!(
                    stars.brightness_key(i - 1) < stars.brightness_key(i)
                        || (stars.brightness_key(i - 1) == stars.brightness_key(i) && stars.id(i - 1) > stars.id(i))
                );
            }
        }
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let projected = crate::projection::project_sky(
            &sky,
            &crate::projection::View::default(),
            crate::projection::Viewport { height: 41, width: 81 },
        );
        let drawn: Vec<_> = projected.stars.iter().map(|s| s.star.id.0).collect();
        assert_eq!(&drawn[..3], &[1894, 365, 3313]);
        assert_eq!(
            drawn.iter().rev().take(3).copied().collect::<Vec<_>>(),
            [2491, 2326, 5340]
        );
    }

    #[test]
    fn stars_keep_their_names_and_spectral_types() {
        let sky = build_sky();
        let star = |id| sky.stars.iter().find(|star| star.id.0 == id).unwrap();
        assert_eq!(
            (sky.star_name(star(2061)), &star(2061).spectral_type),
            (Some("Betelgeuse"), b"M1")
        );
        assert_eq!(
            (sky.star_name(star(5340)), &star(5340).spectral_type),
            (Some("Arcturus"), b"K1")
        );
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert!(!sky.stars.iter().any(|star| star.id.0 == 92)); // HR 92 has no data
    }

    #[test]
    fn constellations_are_matched_by_hr_number_in_any_dataset() {
        let star = |hr: Option<u32>| CatalogStar {
            id: crate::catalog::StarId(u64::from(hr.unwrap_or(100))),
            space_motion: None,
            hr,
            name: None,
            designation: None,
            right_ascension: 0.0,
            declination: 0.0,
            ra_motion: 0.0,
            ra_motion_cos_dec: 0.0,
            dec_motion: 0.0,
            magnitude: 1.0,
            spectral_type: *b"  ",
            color_index: None,
            has_data: true,
        };
        let figure = |abbreviation, segments| ConstellationFigure { abbreviation, segments };
        let catalog = Catalog::new(
            vec![star(Some(30)), star(None), star(Some(10)), star(Some(20))],
            StarNames::default(),
            vec![
                figure("Abc", vec![[10, 20], [20, 99]]), // HR 99 isn't in the dataset
                figure("Def", vec![[98, 99]]),
            ],
        );
        let sky = Sky::from_catalog(&catalog);
        assert_eq!(sky.constellations.len(), 1);
        let [a, b] = sky.constellations[0].segments[0];
        assert_eq!((sky.stars[a].id.0, sky.stars[b].id.0), (10, 20));
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations.len(), 88);
        let [a, b] = sky.constellations[19].segments[0];
        assert_eq!((sky.stars[a].id.0, sky.stars[b].id.0), (4785, 4915));
    }
}
