//! Four-stage pipeline: immutable catalog and independently cached geometric simulation -> observer-relative sky
//! -> camera projection -> rendering. Astronomy families live in astro::models; no camera enters observation.
//! Common states use f64 J2000 equatorial AU/AU-day, currently with a heliocentric origin. Earth is the only real
//! anchor. Its site vector remains zero; the legacy Moon altitude parallax is applied in observation exactly once.

mod illumination;
mod objects;
pub use illumination::{MoonIllumination, compute_moon_illumination};
mod observation;
mod positions;
pub mod simulation;
pub use observation::{Anchor, ObserverState, observe_sky, prepare_observer};
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
    /// Brightest first; equal magnitudes by descending stable ID. Placeholders are omitted.
    pub stars: Vec<Star>,
    pub names: StarNames,
    pub constellations: Vec<Constellation>,
}

impl SkyCatalog {
    /// Prepare a sorted immutable catalog and resolve constellation endpoints.
    pub fn from_catalog(catalog: &Catalog) -> SkyCatalog {
        // compact and sort drawable stars, preserving the old drawing and label tie order
        let mut stars: Vec<Star> = catalog
            .stars
            .iter()
            .filter(|star| star.has_data)
            .map(Star::from_catalog_star)
            .collect();
        stars.sort_unstable_by(|a, b| a.magnitude.total_cmp(&b.magnitude).then_with(|| b.id.cmp(&a.id)));

        // resolve representatives after sorting, without changing the choice made before overrides
        let hr_by_id: HashMap<_, _> = catalog.hr_representatives.iter().map(|(&hr, &id)| (id, hr)).collect();
        let index_by_hr: HashMap<u32, usize> = stars
            .iter()
            .enumerate()
            .filter_map(|(index, star)| Some((*hr_by_id.get(&star.id)?, index)))
            .collect();
        let constellations = catalog
            .constellations
            .iter()
            .filter_map(|figure| resolve_constellation_figure(figure, &index_by_hr))
            .collect();

        SkyCatalog {
            stars,
            names: catalog.names.clone(),
            constellations,
        }
    }

    /// Length of the drawable prefix for an inclusive magnitude threshold.
    pub fn count_bright_stars(&self, threshold: f32) -> usize {
        self.stars.partition_point(|star| star.magnitude <= threshold)
    }

    /// Resolve a star's name from the sky-owned string block.
    pub fn star_name(&self, star: &Star) -> Option<&str> {
        self.names.get(star.name)
    }
}

/// Read-only output of observation, independent of a view. Shares immutable catalog data across sites and frames.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedSky {
    pub catalog: Arc<SkyCatalog>,
    pub stars: Vec<ObservedStar>,
    pub planets: Vec<Planet>,
    pub moon: Moon,
    pub names: crate::catalog::StarNames,
    pub constellations: Vec<Constellation>,
    pub(crate) refracted: bool,
}
/// Compatibility name for the observed sky; simulation caches are a separate type.
pub type Sky = ObservedSky;

impl ObservedSky {
    pub fn new(catalog: Arc<SkyCatalog>) -> Self {
        Self {
            names: catalog.names.clone(),
            constellations: catalog.constellations.clone(),
            catalog,
            stars: Vec::new(),
            planets: create_planets(),
            moon: create_moon(),
            refracted: false,
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
            .map(|star| ObservedStar::from_star(star, crate::astro::Horizontal::default()))
            .collect();
        sky
    }
    pub fn count_bright_stars(&self, threshold: f32) -> usize {
        self.stars.partition_point(|star| star.magnitude <= threshold)
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
        assert!(sky.stars.windows(2).all(|pair| pair[0].magnitude < pair[1].magnitude
            || (pair[0].magnitude == pair[1].magnitude && pair[0].id > pair[1].id)));
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let drawn: Vec<_> = sky.stars.iter().rev().map(|star| star.id.0).collect();
        assert_eq!(&drawn[..3], &[1894, 365, 3313]);
        assert_eq!(
            sky.stars[..3].iter().map(|star| star.id.0).collect::<Vec<_>>(),
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
