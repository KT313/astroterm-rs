//! Independent owners and restricted borrows for simulation, observation, projection and scene drawing.
mod simulation;
mod observation;
mod projection;
mod scene;
pub use simulation::SimulationState;
pub use observation::ObservationCache;
pub use projection::ProjectionCache;
pub use scene::SceneCache;
pub(crate) use observation::{
    RegionCache, CandidateCache, SelectedCache, WorkingCache, MotionCache, EligibleCache, RelativeCache,
    IlluminationCache, ApparentCache, HorizontalCache, StellarMotionBuffers, ObserverBuffers, LightTimeBuffers,
};
pub(crate) use projection::{StarProjectionBuffers, DrawOrderBuffers};
