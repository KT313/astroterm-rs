//! The sky model: every object in the sky and its apparent position. It holds no rendering details, so any renderer
//! can draw it.

mod objects;
mod positions;

pub use objects::{Constellation, Moon, Planet, PlanetKind, Star, create_moon, create_planets};
pub use positions::{refract_sky_positions, update_sky_positions};

use std::collections::HashMap;

use crate::catalog::{Catalog, ConstellationFigure};

/// All objects in the sky.
#[derive(Clone, Debug)]
pub struct Sky {
    /// In catalog order; for the embedded catalog, star `i` has HR number `i + 1`.
    pub stars: Vec<Star>,
    /// Indices of the stars that have catalog data, dimmest first, so renderers can draw brighter stars on top.
    pub stars_by_brightness: Vec<usize>,
    /// The Sun and planets, ordered from the Sun outwards.
    pub planets: Vec<Planet>,
    pub moon: Moon,
    pub constellations: Vec<Constellation>,
}

impl Sky {
    /// Build the sky from the parsed catalogs. Positions are zero until updated.
    pub fn from_catalog(catalog: &Catalog) -> Sky {
        // stars, and the order to draw them in
        let stars: Vec<Star> = catalog.stars.iter().map(Star::from_catalog_star).collect();
        let stars_by_brightness = sort_stars_dimmest_first(&stars);

        // constellation figures, with their HR numbers resolved to star indices
        let index_by_hr: HashMap<u32, usize> = (catalog.stars.iter().enumerate())
            .filter_map(|(index, star)| Some((star.hr?, index)))
            .collect();
        let constellations = catalog
            .constellations
            .iter()
            .filter_map(|figure| resolve_constellation_figure(figure, &index_by_hr))
            .collect();

        Sky {
            stars,
            stars_by_brightness,
            planets: create_planets(),
            moon: create_moon(),
            constellations,
        }
    }

    /// The Sun, which is the first entry of `planets`.
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

/// Indices of the stars with data, sorted by decreasing magnitude (dimmest first).
fn sort_stars_dimmest_first(stars: &[Star]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..stars.len()).filter(|&index| stars[index].has_data).collect();
    indices.sort_by(|&a, &b| stars[b].magnitude.total_cmp(&stars[a].magnitude));
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogStar, ConstellationFigure, Designation, load_embedded_catalog};

    fn build_sky() -> Sky {
        Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"))
    }

    #[test]
    fn stars_are_indexed_by_catalog_number() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110);
        assert_eq!(sky.stars[7000].name, Some("Vega"));
        let is_hr = |index: usize, star: &Star| star.designation == Some(Designation::Hr(index as u32 + 1));
        assert!(sky.stars.iter().enumerate().all(|(index, star)| is_hr(index, star)));
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let catalog_number = |index: usize| sky.stars_by_brightness[index] + 1; // HR number
        let last = sky.stars_by_brightness.len() - 1;
        assert_eq!(
            [catalog_number(0), catalog_number(1), catalog_number(2)],
            [1894, 365, 3313]
        );
        assert_eq!(
            [catalog_number(last), catalog_number(last - 1), catalog_number(last - 2)],
            [2491, 2326, 5340]
        );
    }

    #[test]
    fn stars_keep_their_names_and_spectral_types() {
        let sky = build_sky();
        let star = |catalog_number: usize| &sky.stars[catalog_number - 1];
        assert_eq!(
            (star(2061).name, &star(2061).spectral_type),
            (Some("Betelgeuse"), b"M1")
        );
        assert_eq!((star(5340).name, &star(5340).spectral_type), (Some("Arcturus"), b"K1"));
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars_by_brightness.len(), 9110 - 14);
        assert!(!sky.stars_by_brightness.contains(&91)); // HR 92 has no data
    }

    #[test]
    fn constellations_are_matched_by_hr_number_in_any_dataset() {
        let star = |hr| CatalogStar {
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
        let catalog = Catalog {
            stars: vec![star(Some(30)), star(None), star(Some(10)), star(Some(20))],
            constellations: vec![
                figure("Abc", vec![[10, 20], [20, 99]]), // HR 99 isn't in the dataset
                figure("Def", vec![[98, 99]]),
            ],
        };
        let sky = Sky::from_catalog(&catalog);
        assert_eq!(sky.constellations.len(), 1);
        assert_eq!(sky.constellations[0].segments, [[2, 3]]);
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations.len(), 88);
        assert_eq!(sky.constellations[19].segments, [[4784, 4914]]);
    }
}
