//! Immutable catalog records, spatial indexing and packed storage.
mod records;
mod grid;
mod storage;
pub use records::{SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException};
pub use grid::{GRID_DEPTH, CELL_COUNT, REFRACTION_MARGIN, ABERRATION_MARGIN, SkyRegion, SelectionStats, SkyGrid, hash_direction};
pub use storage::{QUANTIZATION_MARGIN, STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage};
pub(crate) use grid::{SelectedRegion, build_caps};

#[cfg(test)]
pub(crate) use grid::{interleave, direction};

#[cfg(feature = "memory-diagnostics")]
pub(crate) use grid::CellCap;

pub(crate) use records::unsupported_star_data;
