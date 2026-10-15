//! Immutable catalog records, spatial indexing and packed storage.
mod records;
mod grid;
mod groups;
mod storage;
mod star_color;
mod stars;
mod constellations;
mod bodies;
mod tables;
use crate::model::{StarRowVec, StarRowSlice};

pub use records::{SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, Star, Constellation};
pub use star_color::StarColor;
pub use grid::{SkyGrid, hash_direction};
pub use groups::{speed_group, get_ttl_of_speed_group_ms, drift_bound_for_rate};
pub(crate) use grid::region_caps;
#[cfg(test)]
pub(crate) use stars::fixtures;
pub use storage::{STAR_SECTIONS, StarRow, StarStorage};
pub(crate) use grid::{CellCap, build_caps};

#[cfg(test)]
pub(crate) use grid::{interleave, direction};

pub(crate) use records::unsupported_star_data;
