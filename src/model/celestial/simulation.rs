//! Time, sampling and error records independent of simulation storage.
use crate::rows::row_columns;
use crate::constants::{PLANETARY_SAMPLE_TTL_SECONDS, LUNAR_SAMPLE_TTL_SECONDS, ORIENTATION_SAMPLE_TTL_SECONDS,
    PLANET_POSITION_ERROR_AU, PLANET_VELOCITY_ERROR_AU_DAY, MOON_POSITION_ERROR_AU, MOON_VELOCITY_ERROR_AU_DAY, ORIENTATION_ERROR_ARCSECONDS};
use crate::astro::{COMPUTATIONAL_INTERVAL, models::BodyId};
use std::fmt;

/// UTC input approximates UT1; TT includes the Espenak–Meeus estimate of ΔT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    pub utc: f64,
    pub ut1: f64,
    pub tt: f64,
}
impl FrameTime {
    pub fn from_utc(utc: f64) -> Self {
        Self {
            utc,
            ut1: utc,
            tt: crate::astro::ut1_to_tt(utc),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFamily {
    Planets,
    Moon,
    Orientation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateRequest {
    pub body: BodyId,
    pub tt: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimulationError {
    InvalidTime,
    MissingCoverage { family: ModelFamily, tt: f64 },
    TooManyEpochs(ModelFamily),
    NonFiniteState(ModelFamily),
}
impl fmt::Display for SimulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "simulation state unavailable: {self:?}")
    }
}
impl std::error::Error for SimulationError {}

/// Per-family interpolation limits, measured against direct model evaluation. These do not include the physical
/// theory's error. Lunar limits are parent-relative; common-frame composition adds the parent's error. For the
/// Earth observer, the same-time parent position cancels before the direction is formed.
#[derive(Clone, Copy, Debug)]
pub struct InterpolationLimits {
    pub position_au: f64,
    pub velocity_au_day: f64,
    pub orientation_arcseconds: f64,
}
pub const PLANET_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: PLANET_POSITION_ERROR_AU,
    velocity_au_day: PLANET_VELOCITY_ERROR_AU_DAY,
    orientation_arcseconds: 0.0,
};
pub const MOON_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: MOON_POSITION_ERROR_AU,
    velocity_au_day: MOON_VELOCITY_ERROR_AU_DAY,
    orientation_arcseconds: 0.0,
};
pub const ORIENTATION_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: 0.0,
    velocity_au_day: 0.0,
    orientation_arcseconds: ORIENTATION_ERROR_ARCSECONDS,
};

/// Sampled interpolation policy, days either side of the sample. Outside the computational interval only an exact
/// sample is accepted. Bounds are qualified by the cadence sweep, separate from physical accuracy targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachePolicy {
    pub planets_days: f64,
    pub moon_days: f64,
    pub orientation_days: f64,
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            planets_days: PLANETARY_SAMPLE_TTL_SECONDS / 86400.0,
            moon_days: LUNAR_SAMPLE_TTL_SECONDS / 86400.0,
            orientation_days: ORIENTATION_SAMPLE_TTL_SECONDS / 86400.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshCounts {
    pub planets: u64,
    pub moon: u64,
    pub orientation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sample<T> {
    pub(crate) epoch: f64,
    pub(crate) half_span: f64,
    pub(crate) value: T,
}
row_columns!(Sample<T> { epoch, half_span, value });
impl<T> Sample<T> {
    pub(crate) fn covers(&self, tt: f64) -> bool {
        tt == self.epoch || (COMPUTATIONAL_INTERVAL.contains(tt) && (tt - self.epoch).abs() <= self.half_span)
    }
}

#[cfg(feature = "memory-diagnostics")]
impl<T: crate::cache::ReportBuffers> crate::cache::ReportBuffers for Sample<T> {
    const HAS_BUFFERS: bool = T::HAS_BUFFERS;
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        crate::cache::report_field(sink, "sample", &self.value);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectedStar {
    pub source_index: usize,
    pub drawable: bool,
}
row_columns!(SelectedStar { source_index, drawable });

#[derive(Debug)]
pub(crate) struct StellarWork {
    pub(crate) magnitude: f64,
    pub(crate) motion: crate::astro::models::stars::StellarMotion,
    pub(crate) class: crate::astro::models::stars::StellarClass,
    pub(crate) sample: Option<crate::astro::models::stars::StellarSample>,
}
row_columns!(StellarWork { magnitude, motion, class, sample });
