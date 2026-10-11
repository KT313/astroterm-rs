//! Observation orchestration over state-owned buffers. Each correction retains its own output; no corrected vector becomes a model input.
//! Stars end at their regional apparent directions; the horizon rotation and refraction are applied when they are read.
mod diagnostics;
mod inputs;
pub(super) use inputs::prepare_inputs;
mod regions;
mod corrections;
use crate::sky::{snapshot_cache, record_cache};
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
use crate::state::{
    ObservationCache,
    RelativeCache, IlluminationCache, HorizontalCache, BodyApparentCache,
};
use super::stages::*;
use crate::model::{ObservedSky, ObserverState};
use crate::astro::Vector3;
use crate::timing::StepTimes;
use crate::cache::{Cache, CacheConfig, Group};

use crate::model::{BodyDirections, ObservationBodyKey as BodyKey, BodySamples};

pub(super) use diagnostics::{capture_observation_reports, describe_observation_results};
pub(super) use regions::{update_regional_brightness, update_regional_corrections, update_regional_aberration};
pub(super) use corrections::{update_observer_subtraction, update_moon_illumination, update_horizon_rotation, update_refraction};

#[cfg(test)]
mod tests;

#[cfg(test)] mod regional_tests;

#[cfg(test)] mod horizon_tests;
