//! Physical records and identities. Calculations are implemented in their formula families.
use crate::formulas::constants::UNREFRACTION_STEPS;

// Coordinates and rotations.
/// Position on the local sky of an observer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Horizontal {
    pub azimuth: f64,
    pub altitude: f64,
}

/// Position on the celestial sphere.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Equatorial {
    pub right_ascension: f64,
    pub declination: f64,
}

/// Geographic location of the observer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Observer {
    pub latitude: f64,
    pub longitude: f64,
}

/// Rectangular coordinates, e.g. heliocentric or geocentric equatorial coordinates in AU.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Row-major 3 × 3 matrix acting on column vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix3(pub [[f64; 3]; 3]);

/// Rotation from the mean equator and equinox of J2000 to those of a date.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrecessionMatrix(pub(super) [[f64; 3]; 3]);

/// The inverse-refraction table of `unrefract_direction`, resolved once by a caller that undoes refraction for
/// many directions; `Unrefraction::unrefract` then reads it without the shared table's initialisation check.
#[derive(Clone, Copy)]
pub struct Unrefraction {
    pub(super) table: &'static [[f64; 2]; UNREFRACTION_STEPS + 1],    // the shared table, built on first use
}

// Body identity and observer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyId {
    Sun,
    Mercury,
    Venus,
    Earth,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Moon,
}

impl BodyId {
    /// Lowercase name, used for per-body table paths.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sun => "sun", Self::Mercury => "mercury", Self::Venus => "venus", Self::Earth => "earth", Self::Mars => "mars",
            Self::Jupiter => "jupiter", Self::Saturn => "saturn", Self::Uranus => "uranus", Self::Neptune => "neptune", Self::Moon => "moon",
        }
    }
}

/// Geometric J2000 equatorial state, AU and AU/day. The common origin is the solar-system barycenter. Lunar samples are explicitly parent-relative until composition.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyState {
    pub position: Vector3,
    pub velocity: Vector3,
}

/// A body's barycentric state together with its acceleration, AU, AU/day and AU/day²: enough to move the state
/// across a light time by a second-order expansion instead of evaluating the series again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyMotion {
    pub position: Vector3,
    pub velocity: Vector3,
    pub acceleration: Vector3,
}

/// The VSOP87E terms of `VSOP87E_TERMS`, decoded once into separate arrays in the embedded order: term `i`
/// contributes `amplitudes[i]·cos(phases[i] + frequencies[i]·t)`.
pub struct SeriesTerms {
    pub amplitudes: Vec<f64>,
    pub phases: Vec<f64>,
    pub frequencies: Vec<f64>,
}

/// Observer's barycentric state and rotations between J2000 and local East/North/Up axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObserverState {
    pub observer: BodyState,
    pub to_horizon: Matrix3,
    pub to_catalog: Matrix3,
}

// Stellar motion and calculated samples.
/// Catalog-only motion properties. Epoch-dependent singular handling remains in evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StellarClass(pub(super) u8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StellarMotion {
    pub u0: Vector3,
    pub w: Vector3,
    pub distance_pc: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StellarSample {
    pub direction: Vector3,
    pub magnitude: f64, // physical magnitude; the caller encodes it when storing a catalog sample
    pub used_singular_fallback: bool,
}

// Lunar illumination and named phases.
/// The eight named phases of the Moon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoonPhase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

/// How much of the Moon is lit and whether it is waxing, from the geometry alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonIllumination {
    pub illuminated_fraction: f64,
    pub phase_angle: f64,
    pub waxing: bool,
}

impl Default for MoonIllumination {
    fn default() -> Self {
        Self { illuminated_fraction: 0.0, phase_angle: std::f64::consts::PI, waxing: true }
    }
}

// Astronomical dates and accuracy categories.
/// Temporary time scales derived at an astronomy boundary, never a stored application timestamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AstronomicalTime {
    pub utc_jd: f64,
    pub ut1_jd: f64,
    pub tt_jd: f64,
}

/// A half-open interval of TT Julian dates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JulianDateInterval {
    pub start_tt: f64,
    pub end_tt: f64,
}

/// Accuracy categories, independent of the theory used to compute them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectClass {
    Stars,
    SunAndPlanets,
    Moon,
}
