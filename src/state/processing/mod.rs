//! Independent stage owners and restricted borrows. Algorithms live outside the state layer.
mod simulation;
mod stellar;
mod observer;
mod selection;
mod observation;
mod projection;
mod scene;
mod support;

pub use simulation::{SimulationState, SimulationCaches};
pub use stellar::{StellarSimulationState, StellarResults};
pub use observer::{ObserverPreparationCache, PreparedBodies};
pub use selection::{StarSelectionCache, SelectedStars};
pub use observation::{ObservationCache, RegionalObservation, ApparentDirections};
pub use projection::ProjectionCache;
pub use scene::SceneCache;

pub(crate) use stellar::{StellarPublication, StellarRegions, StellarMotionBuffers};
pub(crate) use selection::{RegionCache, WorkingCache};
pub(crate) use observer::{ObserverBuffers, LightTimeBuffers};
pub(crate) use observation::{RelativeCache, IlluminationCache, HorizontalSources, HorizontalCache};
pub(crate) use projection::{StarProjectionBuffers, DrawOrderBuffers};
use support::{StageId, sum_stats};
