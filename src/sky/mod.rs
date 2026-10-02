//! The sky model: every object that is drawn, and their apparent positions.

mod objects;
mod positions;

pub use objects::{Appearance, Constellation, Moon, Planet, Star, create_moon, create_planets};
pub use positions::{update_moon, update_planet_positions, update_star_positions};

use crate::catalog::Catalog;

/// All objects in the sky.
#[derive(Clone, Debug)]
pub struct Sky {
    /// Indexed by `catalog_number - 1`.
    pub stars: Vec<Star>,
    /// Indices of the stars that have catalog data, dimmest first, so brighter stars are drawn on top.
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
        let stars: Vec<Star> = catalog
            .stars
            .iter()
            .zip(&catalog.star_names)
            .map(|(entry, name)| Star::from_entry(entry, *name))
            .collect();
        let stars_by_brightness = sort_stars_dimmest_first(&stars);

        // constellation figures referencing stars by table index
        let constellations = catalog
            .constellations
            .iter()
            .map(|figure| Constellation {
                abbreviation: figure.abbreviation,
                segments: figure
                    .segments
                    .iter()
                    .map(|&[a, b]| [a as usize - 1, b as usize - 1])
                    .collect(),
            })
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

/// Indices of the stars with data, sorted by decreasing magnitude (dimmest first).
fn sort_stars_dimmest_first(stars: &[Star]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..stars.len()).filter(|&index| stars[index].has_data).collect();
    indices.sort_by(|&a, &b| stars[b].magnitude.total_cmp(&stars[a].magnitude));
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Color;
    use crate::catalog::load_embedded_catalog;

    fn build_sky() -> Sky {
        Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"))
    }

    #[test]
    fn stars_are_indexed_by_catalog_number() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110);
        assert_eq!(sky.stars[7000].appearance.label, Some("Vega"));
        assert!(
            sky.stars
                .iter()
                .enumerate()
                .all(|(index, star)| star.catalog_number as usize == index + 1)
        );
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let catalog_number = |index: usize| sky.stars[sky.stars_by_brightness[index]].catalog_number;
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
    fn bright_stars_get_their_spectral_colors() {
        let sky = build_sky();
        let star = |catalog_number: usize| &sky.stars[catalog_number - 1].appearance;
        assert_eq!(
            (star(2061).label, star(2061).color),
            (Some("Betelgeuse"), Some(Color::Red))
        );
        assert_eq!((star(1713).label, star(1713).color), (Some("Rigel"), Some(Color::Cyan)));
        assert_eq!(
            (star(5340).label, star(5340).color),
            (Some("Arcturus"), Some(Color::Yellow))
        );
        assert_eq!((star(7001).label, star(7001).color), (Some("Vega"), None));
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars_by_brightness.len(), 9110 - 14);
        assert!(!sky.stars_by_brightness.contains(&91)); // HR 92 has no data
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations.len(), 88);
        assert_eq!(sky.constellations[19].segments, [[4784, 4914]]);
    }
}
