//! Immutable prepared catalog data.
use crate::model::{Star, StarStorage, Constellation, SkyGrid};
use crate::catalog::StarNames;

/// Immutable model inputs and display metadata, shared by all observed skies.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyCatalog {
    /// Compact immutable inputs, sorted by cell then conservative brightness key and descending stable ID.
    pub stars: StarStorage,
    pub grid: SkyGrid,
    /// Sorted union of all constellation endpoints, independent of candidate selection.
    pub endpoint_indices: crate::catalog::cache::CatalogArray<usize>,
    /// Indices whose trajectory drift exceeds the grid-margin threshold.
    pub always_checked: crate::catalog::cache::CatalogArray<usize>,
    pub singular_count: usize,
    pub names: StarNames,
    pub constellations: Vec<Constellation>,
}

impl SkyCatalog {
    /// A valid catalog with no stars, figures or names: the state root starts from it so every owner exists with
    /// its final type before the dataset is loaded. Equal to preparing an empty parsed catalog.
    pub fn empty() -> Self {
        Self {
            stars: StarStorage::default(),
            grid: SkyGrid::from_offsets(vec![0; crate::model::CELL_COUNT + 1].into()),
            endpoint_indices: Default::default(),
            always_checked: Default::default(),
            singular_count: 0,
            names: StarNames::default(),
            constellations: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_catalog_matches_preparing_no_rows() {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(Vec::new(), StarNames::default(), Vec::new()));
        assert_eq!(SkyCatalog::empty(), prepared);
        assert!(SkyCatalog::empty().stars.is_empty());
    }
}
