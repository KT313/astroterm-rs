//! Shared records: the catalog data model of the sky tables and the validated configuration.
//! Each submodule exposes its public symbols here; callers use `crate::model::X`.
mod catalog;
mod configuration;

pub use catalog::{
    SkyCatalog, CatalogPreparation, PreparedCatalog, ConstellationSet, StarException, SkyGrid,
    hash_direction, speed_group, speed_group_hold_seconds, drift_bound_for_rate, STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec, StarStorage, Star, Constellation, StarColor,
    Stars, GroupTable, LabelColumn, Constellations, Bodies, Body, Satellite, SkyTables,
};
pub(crate) use catalog::{unsupported_star_data, CellCap, build_caps, region_caps};
#[cfg(test)]
pub(crate) use catalog::fixtures;
pub use configuration::{FrameBenchmarkConfig, Config, SimulationSettings, TerminalSettings, GraphicsProtocol,
    ViewCenter, ProjectionKind, View, Cell, ProjectionViewport, RenderOptions};
#[cfg(test)]
pub(crate) use catalog::{interleave, direction};
