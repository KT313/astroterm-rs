//! Application ownership root and narrow stage views. Import owners through `crate::state`.
//! See README.md for producers, consumers, units and reset rules; processing algorithms stay outside this module.

mod application;
mod processing;
mod rendering;
mod tables;

pub use application::{ApplicationState, Persistent, Caches};
pub use processing::{
    SimulationCaches, SimulationState, StellarSimulationState, ObserverPreparationCache, StarSelectionCache,
    SelectedStars, StellarResults, PreparedBodies, ObservationCache, RegionalObservation, ProjectionCache, SceneCache,
};
pub(crate) use processing::{
    ObserverBuffers, LightTimeBuffers, RegionCache, WorkingCache,
    RelativeCache, IlluminationCache, HorizontalCache, BodyApparentCache, StellarPublication, StellarRegions, StellarMotionBuffers,
    StarProjectionBuffers, DrawOrderBuffers,
};
pub use rendering::{RenderingState, CompressionSupport, CharacterState, PixelState, Presenter, TextRasterizer};
pub use tables::{Table, TableBytes, TableVisitor, Tables};

pub(crate) use rendering::SharedMemoryImage;
