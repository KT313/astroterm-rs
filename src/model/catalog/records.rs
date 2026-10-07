//! Immutable prepared catalog data.
use crate::model::{Star, StarStorage, Constellation, SkyGrid};
use crate::catalog::StarNames;
use std::sync::Arc;

/// Immutable model inputs and display metadata, shared by all observed skies.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyCatalog {
    /// Compact immutable inputs, sorted by cell then conservative brightness key and descending stable ID.
    pub stars: StarStorage,
    pub star_exceptions: Vec<StarException>,
    pub grid: SkyGrid,
    pub singular_count: usize,
    pub names: StarNames,
    pub figures: Arc<ConstellationSet>,
}

impl SkyCatalog {
    /// Stage-3 boundary: reject exceptional catalogs until sparse processing is implemented.
    pub fn validate_exception_support(&self) -> std::io::Result<()> {
        if let Some(entry) = self.star_exceptions.first() {
            return Err(unsupported_star_data(&format!("catalog has {} star exception(s), beginning at catalog row {}", self.star_exceptions.len(), entry.catalog_row_index)));
        }
        self.stars.validate_exception_storage()
    }

    pub fn constellations(&self) -> &[Constellation] { self.figures.figures() }
    pub fn endpoint_indices(&self) -> &[usize] { self.figures.endpoints() }

    /// A valid catalog with no stars, figures or names: the state root starts from it so every owner exists with
    /// its final type before the dataset is loaded. Equal to preparing an empty parsed catalog.
    pub fn empty() -> Self {
        Self {
            stars: StarStorage::default(),
            star_exceptions: Vec::new(),
            grid: SkyGrid::from_offsets(vec![0; crate::model::SIMULATION_REGION_COUNT + 1].into()),
            singular_count: 0,
            names: StarNames::default(),
            figures: Arc::new(ConstellationSet::default()),
        }
    }

    /// Number of possible-brightness candidates across all cells for an inclusive threshold.
    pub fn count_bright_stars(&self, threshold: f64) -> usize {
        self.stars.brightness_keys().iter().filter(|&&key| crate::catalog::passes_brightness_bound(key, threshold)).count()
    }

    /// Resolve a star's name from the original catalog-owned string block.
    pub fn star_name(&self, star: &Star) -> Option<&str> {
        self.names.get(star.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_catalog_matches_preparing_no_rows() {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(Vec::new(), StarNames::default(), Vec::new())).unwrap();
        assert_eq!(SkyCatalog::empty(), prepared.catalog);
        assert!(SkyCatalog::empty().stars.is_empty());
    }
}

/// Startup-only data in the same row order as the completed runtime catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogPreparation {
    pub(crate) motion_bounds: Vec<f32>,
}
impl CatalogPreparation {
    pub fn motion_bounds(&self) -> &[f32] { &self.motion_bounds }
}

/// Transfer validated runtime inputs and their disposable preparation payload together.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedCatalog {
    pub catalog: SkyCatalog,
    pub preparation: CatalogPreparation,
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(CatalogPreparation { motion_bounds });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(PreparedCatalog { catalog, preparation });

/// One immutable definition set, shared by catalog users and retained cache keys.
#[derive(Debug, Default, PartialEq)]
pub struct ConstellationSet {
    pub(crate) figures: Vec<Constellation>,
    pub(crate) endpoints: Vec<usize>,
}
impl ConstellationSet {
    pub fn figures(&self) -> &[Constellation] { &self.figures }
    pub fn endpoints(&self) -> &[usize] { &self.endpoints }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(ConstellationSet { figures, endpoints });

/// Sparse future metadata. Row indices address the final sorted catalog; zero precise_motion_entry means absent.
/// No entries are accepted yet, including entries that claim neither exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StarException {
    pub catalog_row_index: usize,
    pub uses_motion_fallback: bool,
    pub precise_motion_entry: u32,
}
crate::rows::row_columns!(StarException { catalog_row_index, uses_motion_fallback, precise_motion_entry });
// Debug column names match the Rust fields. Precision entries are one-based references to stars.precise_motions.

pub(crate) fn unsupported_star_data(reason: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, format!("{reason}; sparse star-exception support is not implemented yet"))
}
