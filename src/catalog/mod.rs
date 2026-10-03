//! Star, constellation, city and orbit data embedded into the binary at compile time, and star datasets loaded from
//! files (AT-HYG) instead of the embedded star catalog.

mod athyg;
mod bsc5;
mod cities;
mod designation;
mod names;
mod orbits;
mod space_motion;
mod tables;

use std::collections::HashMap;
use std::fmt;

pub use athyg::load_athyg_catalog;
pub use bsc5::{Bsc5Entry, parse_bsc5};
pub use cities::{City, find_city, parse_cities, suggest_cities};
pub use designation::Designation;
pub use names::{NameId, StarNames};
pub use orbits::{
    EARTH_ORBIT, JUPITER_ORBIT, MARS_ORBIT, MERCURY_ORBIT, MOON_ORBIT, NEPTUNE_ORBIT, SATURN_ORBIT, URANUS_ORBIT,
    VENUS_ORBIT,
};
pub use space_motion::SpaceMotion;
pub use tables::{ConstellationFigure, parse_constellation_figures, parse_star_names};

const BSC5_DATA: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5"));
const STAR_NAMES_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_names.txt"));
const CONSTELLATIONS_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bsc5_constellations.txt"));
const CITIES_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/cities.csv"));

/// A star catalog, whichever dataset it comes from, with the constellation figures.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    /// For the embedded catalog, star `i` has HR number `i + 1`; other datasets are in their own order.
    pub stars: Vec<CatalogStar>,
    pub names: StarNames,
    /// HR to stable ID, selected before magnitude overrides.
    pub hr_representatives: HashMap<u32, StarId>,
    /// Figures refer to stars by HR number.
    pub constellations: Vec<ConstellationFigure>,
}

/// Identity within a catalog: BSC5 HR number, or zero-based AT-HYG data-row index (including skipped rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StarId(pub u64);

impl Catalog {
    /// Choose each HR representative by original magnitude, then lowest stable ID.
    pub fn new(stars: Vec<CatalogStar>, names: StarNames, constellations: Vec<ConstellationFigure>) -> Self {
        let mut representatives: HashMap<u32, &CatalogStar> = HashMap::new();
        for star in stars.iter().filter(|star| star.has_data) {
            if let Some(hr) = star.hr {
                let previous = representatives.entry(hr).or_insert(star);
                if star.magnitude < previous.magnitude
                    || (star.magnitude == previous.magnitude && star.id < previous.id)
                {
                    *previous = star;
                }
            }
        }
        let hr_representatives = representatives.into_iter().map(|(hr, star)| (hr, star.id)).collect();
        Self {
            stars,
            names,
            hr_representatives,
            constellations,
        }
    }
}

/// A star as any dataset describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogStar {
    pub id: StarId,
    /// Validated 3D inputs, reserved for the future space-motion model.
    pub space_motion: Option<SpaceMotion>,
    /// Harvard Revised / Yale Bright Star Catalogue number, which the constellation figures refer to.
    pub hr: Option<u32>,
    /// Proper name, for the stars that have one.
    pub name: Option<NameId>,
    /// The best catalog designation, used as a label for stars without a name.
    pub designation: Option<Designation>,
    /// J2000 position in radians.
    pub right_ascension: f64,
    pub declination: f64,
    /// Proper motion of the right ascension and declination themselves, in radians per year.
    pub ra_motion: f64,
    /// Tangential RA motion (dRA/dt · cos declination), radians/Julian year; retained even at a pole.
    pub ra_motion_cos_dec: f64,
    pub dec_motion: f64,
    pub magnitude: f32,
    /// Morgan-Keenan spectral class and subclass, e.g. `*b"K1"`; blank if unknown.
    pub spectral_type: [u8; 2],
    /// B-V color index, if known.
    pub color_index: Option<f32>,
    /// Whether the catalog has data for this star (BSC5 keeps a few catalog numbers as empty placeholders).
    pub has_data: bool,
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
    /// A dataset file can't be read.
    Io(String),
    /// A dataset lacks a required column.
    MissingColumn(&'static str),
    /// A dataset row has a value that can't be parsed.
    MalformedDatasetRow {
        line: u64,
        column: &'static str,
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
            CatalogError::Io(message) => f.write_str(message),
            CatalogError::MissingColumn(column) => write!(f, "dataset has no `{column}` column"),
            CatalogError::MalformedDatasetRow { line, column } => {
                write!(f, "malformed `{column}` value on line {line} of the dataset")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Parse the catalogs embedded in the binary: the Yale Bright Star Catalog with its star names.
pub fn load_embedded_catalog() -> Result<Catalog, CatalogError> {
    let entries = parse_bsc5(BSC5_DATA)?;
    let star_names = parse_star_names(STAR_NAMES_TEXT, entries.len())?;
    let mut names = StarNames::default();
    let stars = entries
        .iter()
        .zip(star_names)
        .map(|(entry, name)| convert_bsc5_entry(entry, name.map(|name| names.insert(name))))
        .collect();
    Ok(Catalog::new(stars, names, load_constellation_figures()?))
}

/// The constellation figures embedded in the binary, by HR number.
pub fn load_constellation_figures() -> Result<Vec<ConstellationFigure>, CatalogError> {
    parse_constellation_figures(CONSTELLATIONS_TEXT)
}

/// A BSC5 entry as a catalog star, designated by its HR number.
fn convert_bsc5_entry(entry: &Bsc5Entry, name: Option<NameId>) -> CatalogStar {
    CatalogStar {
        id: StarId(u64::from(entry.catalog_number)),
        space_motion: None,
        hr: Some(entry.catalog_number),
        name,
        designation: Some(Designation::Hr(entry.catalog_number)),
        right_ascension: entry.right_ascension,
        declination: entry.declination,
        ra_motion: entry.ra_motion,
        ra_motion_cos_dec: entry.ra_motion * entry.declination.cos(),
        dec_motion: entry.dec_motion,
        magnitude: entry.magnitude,
        spectral_type: entry.spectral_type,
        color_index: None,
        has_data: entry.has_data(),
    }
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
        assert_eq!(catalog.stars.len(), 9110);
        assert!(
            catalog
                .stars
                .iter()
                .enumerate()
                .all(|(index, star)| star.hr == Some(index as u32 + 1))
        );
        assert_eq!(catalog.names.get(catalog.stars[7000].name), Some("Vega"));

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
