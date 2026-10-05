//! Immutable prepared catalog data.
use crate::model::{Star, StarStorage, Constellation};
use crate::model::grid;
use crate::catalog::StarNames;

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
