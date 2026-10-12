//! Immutable catalog records, spatial indexing and packed storage.
mod records;
mod grid;
mod storage;
mod star_color;
mod stars;
mod constellations;
mod bodies;
mod tables;
pub use stars::{Stars, GroupTable, LabelColumn};
pub use constellations::Constellations;
pub use bodies::{Bodies, Body, Satellite};
pub use tables::SkyTables;
pub use records::{SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, Star, Constellation};
pub use star_color::StarColor;
pub use grid::{SkyRegion, SelectionStats, SkyGrid, hash_direction};
pub(crate) use grid::region_caps;
#[cfg(test)]
pub(crate) use stars::fixtures;
pub use storage::{STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage};
pub(crate) use storage::StellarFields;
pub(crate) use grid::{SelectedRegion, CellCap, build_caps};

#[cfg(test)]
pub(crate) use grid::{interleave, direction};

pub(crate) use records::unsupported_star_data;
