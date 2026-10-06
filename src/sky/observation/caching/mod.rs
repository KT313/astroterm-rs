//! Observation orchestration over state-owned buffers. Each correction retains its own output; no corrected vector becomes a model input.
mod diagnostics;
mod stellar;
mod preparation;
mod selection;
mod corrections;
use super::memory::{snapshot_cache, record_cache, record_observer_memory, record_direction_pass, record_direction_capture, record_direction_restoration};
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use crate::state::{
    ObservationCache, RegionCache, CandidateCache, SelectedCache, WorkingCache, MotionCache, EligibleCache,
    RelativeCache, IlluminationCache, ApparentCache, HorizontalCache, ObserverBuffers, LightTimeBuffers,
};
use super::stages::*;
use crate::model::{ObservedSky, ObserverState, FrameTime, SimulationError};
use crate::astro::{Observer, Vector3};
use crate::state::SimulationState;
use crate::timing::StepTimes;
use crate::sky::refract_sky_positions;
use crate::astro::models::stars::{StellarMotion, StellarSample, years_since_j2000};
use crate::cache::{Cache, CacheConfig, Group};
use crate::model::{ObservedStar, SkyCatalog};
use std::sync::Arc;

#[cfg(test)]
use stellar::qualify_stellar_span;

use crate::model::{Directions, ObservationBodyKey as BodyKey, CorrectionSelection, BodySamples};

pub use preparation::{prepare_observation_catalog, prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time};
pub(super) use preparation::reset_catalog_if_changed;
pub(super) use diagnostics::{capture_observation_reports, describe_observation_results};
pub(super) use stellar::refresh_stellar_motion as update_stellar_motion;
pub(super) use selection::{update_region_filtering, update_brightness_bounds, update_candidate_validation, update_constellation_endpoints, update_current_brightness, update_correction_selection};
pub(super) use corrections::{update_body_sampling, update_observer_subtraction, update_moon_illumination, update_aberration, update_horizon_rotation, update_refraction};

#[cfg(test)]
mod tests;
