//! Star, constellation, city and orbit data embedded into the binary at compile time.

mod bsc5;
mod cities;
mod orbits;
mod tables;

use std::fmt;

pub use bsc5::{Bsc5Entry, parse_bsc5};
pub use cities::{City, find_city, parse_cities};
pub use orbits::{
    EARTH_ORBIT, JUPITER_ORBIT, MARS_ORBIT, MERCURY_ORBIT, MOON_ORBIT, NEPTUNE_ORBIT, SATURN_ORBIT, URANUS_ORBIT,
    VENUS_ORBIT,
};
pub use tables::{ConstellationFigure, parse_constellation_figures, parse_star_names};

const BSC5_DATA: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5"));
const STAR_NAMES_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_names.txt"));
const CONSTELLATIONS_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_constellations.txt"));
const CITIES_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/cities.csv"));

/// The parsed star catalog with names and constellation figures.
#[derive(Clone, Debug)]
pub struct Catalog {
    /// BSC5 entries; entry `i` has catalog number `i + 1`.
    pub stars: Vec<Bsc5Entry>,
    /// Proper names, indexed like `stars`.
    pub star_names: Vec<Option<&'static str>>,
    pub constellations: Vec<ConstellationFigure>,
}

/// Malformed embedded data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// The BSC5 data ends before the header (`entry: None`) or inside an entry.
    TruncatedBsc5 {
        entry: Option<usize>,
    },
    MalformedStarName {
        line: usize,
    },
    MalformedConstellation {
        line: usize,
    },
    MalformedCity {
        line: usize,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::TruncatedBsc5 { entry: None } => {
                write!(f, "star catalog is too short for its header")
            }
            CatalogError::TruncatedBsc5 { entry: Some(entry) } => {
                write!(f, "star catalog is truncated at entry {entry}")
            }
            CatalogError::MalformedStarName { line } => {
                write!(f, "malformed star name on line {line}")
            }
            CatalogError::MalformedConstellation { line } => {
                write!(f, "malformed constellation on line {line}")
            }
            CatalogError::MalformedCity { line } => write!(f, "malformed city on line {line}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Parse the catalogs embedded in the binary.
pub fn load_embedded_catalog() -> Result<Catalog, CatalogError> {
    let stars = parse_bsc5(BSC5_DATA)?;
    let star_names = parse_star_names(STAR_NAMES_TEXT, stars.len())?;
    let constellations = parse_constellation_figures(CONSTELLATIONS_TEXT)?;
    Ok(Catalog {
        stars,
        star_names,
        constellations,
    })
}

/// Parse the city table embedded in the binary.
pub fn load_embedded_cities() -> Result<Vec<City>, CatalogError> {
    parse_cities(CITIES_TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_consistent() {
        let catalog = load_embedded_catalog().expect("embedded catalog loads");
        assert_eq!(catalog.star_names.len(), catalog.stars.len());

        let star_exists = |number: u32| (1..=catalog.stars.len() as u32).contains(&number);
        let all_segments = catalog
            .constellations
            .iter()
            .flat_map(|figure| figure.segments.iter().flatten());
        assert!(
            all_segments.copied().all(star_exists),
            "every constellation star is in the catalog"
        );
    }

    #[test]
    fn embedded_cities_load() {
        let cities = load_embedded_cities().expect("embedded cities load");
        assert_eq!(cities.len(), 2960);
    }
}
