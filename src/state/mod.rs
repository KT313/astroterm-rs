//! Application ownership root and narrow stage views. Import owners through `crate::state`.
//! See README.md for producers, consumers, units and reset rules; processing algorithms stay outside this module.

mod application;
mod processing;
mod rendering;
mod tables;
#[cfg(feature = "memory-diagnostics")]
mod memory;

pub use application::{ApplicationState, Persistent, Caches};
pub use processing::{
    SimulationCaches, SimulationState, StellarSimulationState, ObserverPreparationCache, StarSelectionCache,
    SelectedStars, StellarResults, PreparedBodies, ObservationCache, RegionalObservation, ApparentDirections, ProjectionCache, SceneCache,
};
pub(crate) use processing::{
    ObserverBuffers, LightTimeBuffers, RegionCache, WorkingCache,
    RelativeCache, IlluminationCache, HorizontalSources, HorizontalCache, StellarPublication, StellarRegions, StellarMotionBuffers,
    StarProjectionBuffers, DrawOrderBuffers,
};
pub use rendering::{RenderingState, CompressionSupport, CharacterState, PixelState, Presenter, TextRasterizer};
pub use tables::{Table, TableBytes, TableVisitor, Tables};
#[cfg(feature = "memory-diagnostics")]
pub(crate) use tables::observation_region_bytes;
#[cfg(feature = "memory-diagnostics")]
pub use memory::{
    InventoryCollector, KnownPayload, collect_inventory, capture_run_inventory, sum_known_payload, write_inventory,
};
