//! Observer and calculated sky records; corrections are applied by sky algorithms.
use std::sync::Arc;
use crate::model::{
    SkyCatalog, SelectionStats, ObservedStar, ObservedStarView, Planet, Moon, Constellation, create_planets,
    create_moon,
};
use std::f64::consts::PI;
use crate::astro::{Observer, Matrix3, Vector3, models::BodyState};
use crate::model::FrameTime;

/// Counts before and after dropping non-drawable stars that are not required constellation endpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CorrectionStats {
    pub evaluated: usize,
    pub skipped: usize,
    pub endpoint_only: usize,
}

/// Read-only output of observation, independent of camera projection. It may cover only the requested SkyRegion;
/// request All when reusing one observation for arbitrary cameras. Catalog data is shared across sites and frames.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedSky {
    pub corrections: CorrectionStats,
    pub magnitude_threshold: f64,
    pub selection: SelectionStats,
    pub(crate) candidate_indices: Vec<usize>,
    pub runtime_singular_count: usize,
    pub catalog: Arc<SkyCatalog>,
    /// Calculated state only. Static display metadata stays packed in `catalog`; use `star_view`/`star_views`.
    pub stars: Vec<ObservedStar>,
    pub planets: Vec<Planet>,
    pub moon: Moon,
    pub names: crate::catalog::StarNames,
    pub constellations: Vec<Constellation>,
    pub(crate) refracted: bool,
    pub outside_accuracy_range: bool,
}
/// Compatibility name for the observed sky; simulation caches are a separate type.
pub type Sky = ObservedSky;

impl ObservedSky {
    pub fn new(catalog: Arc<SkyCatalog>) -> Self {
        Self {
            corrections: CorrectionStats::default(),
            magnitude_threshold: f64::INFINITY,
            selection: SelectionStats::default(),
            candidate_indices: Vec::new(),
            runtime_singular_count: 0,
            names: catalog.names.clone(),
            constellations: catalog.constellations.clone(),
            catalog,
            stars: Vec::new(),
            planets: create_planets(),
            moon: create_moon(),
            refracted: false,
            outside_accuracy_range: false,
        }
    }
    pub fn count_bright_stars(&self, threshold: f64) -> usize {
        self.stars
            .iter()
            .filter(|star| star.drawable && star.magnitude <= threshold)
            .count()
    }
    pub fn star_name(&self, star: &ObservedStar) -> Option<&str> {
        self.names.get(self.catalog.stars.name(star.source_index))
    }
    /// Borrow one calculated record and its catalog metadata; `index` addresses this observed subset.
    pub fn star_view(&self, index: usize) -> ObservedStarView<'_> {
        ObservedStarView {
            state: &self.stars[index],
            catalog: &self.catalog.stars,
        }
    }
    /// Iterate the observed subset without allocating or copying metadata.
    pub fn star_views(&self) -> impl DoubleEndedIterator<Item = ObservedStarView<'_>> + ExactSizeIterator {
        self.stars.iter().map(|state| ObservedStarView {
            state,
            catalog: &self.catalog.stars,
        })
    }
    pub fn sun(&self) -> &Planet {
        &self.planets[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonIllumination {
    pub illuminated_fraction: f64,
    pub phase_angle: f64,
    pub waxing: bool,
}
impl Default for MoonIllumination {
    fn default() -> Self {
        Self {
            illuminated_fraction: 0.0,
            phase_angle: PI,
            waxing: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Earth,
}

/// A frame's observer in the common inertial frame. Site coordinates are body-fixed; full orientation (slow and
/// fast) transforms the WGS84 sea-level site vector and its rotation velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObserverState {
    pub anchor: Anchor,
    pub site: Observer,
    pub height_m: f64,
    pub time: FrameTime,
    pub state: BodyState,
    pub inertial_to_fixed: Matrix3,
    pub inertial_to_horizon: Matrix3,
    pub atmosphere: bool,
    pub emission_tt: [f64; 10],
}

#[derive(Clone, PartialEq)]
pub(crate) struct BodySamples {
    pub(crate) planets: Vec<crate::astro::models::BodyState>,
    pub(crate) moon: crate::astro::models::BodyState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SelectedStar {
    pub source_index: usize,
    pub drawable: bool,
}

#[derive(Clone, PartialEq)]
pub(crate) struct CorrectionSelection {
    pub(crate) indices: Vec<usize>,
    pub(crate) stats: crate::model::CorrectionStats,
}

#[derive(Debug)]
pub(crate) struct StellarWork {
    pub(crate) source_index: usize,
    pub(crate) magnitude: f64,
    pub(crate) refresh: bool,
    pub(crate) motion: Option<crate::astro::models::stars::StellarMotion>,
    pub(crate) class: Option<crate::astro::models::stars::StellarClass>,
    pub(crate) sample: Option<crate::astro::models::stars::StellarSample>,
    pub(crate) valid_seconds: f64,
    pub(crate) calculated_at: f64,
}

#[derive(Default)]
pub(crate) struct ValidityCounts {
    pub(crate) positive: usize,
    pub(crate) outside_interval: usize,
    pub(crate) singular: usize,
    pub(crate) moving_distance: usize,
    pub(crate) zero_limit: usize,
    pub(crate) boundary_or_bound: usize,
    pub(crate) probe_evaluations: usize,
}

pub(crate) type Directions = (Vec<Vector3>, Vec<Vector3>, Vector3);
pub(crate) type ObserverKey = (FrameTime, Observer, [u64; 3], u64, u64);
pub(crate) type BodyKey = (ObserverState, u64, u64);
