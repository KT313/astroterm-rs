//! Shared records: the catalog data model of the sky tables and the validated configuration.
//! Each submodule exposes its public symbols here; callers use `crate::model::X`.
mod data;
mod catalog;
pub use data::{Data, Region, StarGroup, StarTable, StarColumns, StarColumn, StarRowVec, StarRowSlice, StarLabels,
    Body, Satellite, SunTable, PlanetsTable, MoonsTable, SimulationCache, GroupTileKey, GroupTileCache, RegionTileCache, Tile, Texel, StarOpacities};
mod configuration;

pub use catalog::{
    SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, SkyGrid,
    hash_direction, speed_group, get_ttl_of_speed_group, drift_bound_for_rate, STAR_SECTIONS, StarRow, StarStorage, Star, Constellation, StarColor,
};
pub(crate) use catalog::{unsupported_star_data, CellCap, build_caps, region_caps};
#[cfg(test)]
pub(crate) use catalog::fixtures;
pub use configuration::{FrameBenchmarkConfig, Config, SimulationSettings, TerminalSettings, GraphicsProtocol,
    ViewCenter, ProjectionKind, View, Cell, ProjectionViewport, RenderOptions};
#[cfg(test)]
pub(crate) use catalog::{interleave, direction};
