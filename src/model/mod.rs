//! Shared pipeline data and representation accessors, below sky/projection/scene processing.
//! Root re-exports intentionally group catalog, object, observation and grid records. Specialized configuration,
//! simulation, projection and rendering records use their model submodule paths; processing modules do not re-export them.
//! Region inputs consistently use the root `model::SkyRegion` path.
//! Application instances belong to state; headless callers may retain explicit standalone backing storage.

pub mod objects;
pub mod storage;
pub mod catalog;
pub mod observation;
pub mod simulation;
pub mod projection;
pub mod rendering;
pub mod config;
pub mod grid;
pub use objects::{Star, ObservedStar, ObservedStarView, Planet, Moon, PlanetKind, Constellation, create_planets, create_moon};
pub use storage::StarStorage;
pub use catalog::SkyCatalog;
pub use observation::{ObservedSky, Sky, CorrectionStats, ObserverState, Anchor, MoonIllumination};
pub use grid::{SkyGrid, SkyRegion, SelectionStats};

#[cfg(feature = "memory-diagnostics")]
mod memory;

pub mod metadata;
