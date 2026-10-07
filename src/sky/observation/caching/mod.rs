//! Observation orchestration over state-owned buffers. Each correction retains its own output; no corrected vector becomes a model input.
mod diagnostics;
mod inputs;
pub(super) use inputs::prepare_inputs;
mod selection;
mod corrections;
use crate::sky::{snapshot_cache, record_cache};
use super::memory::{ record_direction_pass, record_direction_capture, record_direction_restoration};
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use crate::state::{
    ObservationCache, WorkingCache, MotionCache, EligibleCache,
    RelativeCache, IlluminationCache, ApparentCache, HorizontalCache,
};
use super::stages::*;
use crate::model::{ObservedSky, ObserverState};
use crate::astro::Vector3;
use crate::timing::StepTimes;
use crate::sky::refract_sky_positions;
use crate::cache::{Cache, CacheConfig, Group};
use crate::model::ObservedStar;

use crate::model::{Directions, ObservationBodyKey as BodyKey, CorrectionSelection, BodySamples};

pub(super) use diagnostics::{capture_observation_reports, describe_observation_results};
pub(super) use selection::{update_current_brightness, update_correction_selection};
pub(super) use corrections::{update_observer_subtraction, update_moon_illumination, update_aberration, update_horizon_rotation, update_refraction};

#[cfg(test)]
mod tests;
