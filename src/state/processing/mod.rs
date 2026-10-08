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
pub use observation::{ObservationCache, RegionalObservation};
pub(crate) use observation::ObservationRegion;
pub use projection::ProjectionCache;
pub use scene::SceneCache;

pub(crate) use stellar::{MotionCache, StellarRegions, StellarMotionBuffers};
pub(crate) use selection::{RegionCache, CandidateCache, SelectedCache, WorkingCache};
pub(crate) use observer::{ObserverBuffers, LightTimeBuffers};
pub(crate) use observation::{EligibleCache, RelativeCache, IlluminationCache, ApparentCache, HorizontalCache};
pub(crate) use projection::{StarProjectionBuffers, DrawOrderBuffers};
use support::{StageId, sum_stats};
