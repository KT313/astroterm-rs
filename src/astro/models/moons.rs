//! Meeus chapter 47 (truncated ELP-2000/82), geometric mean ecliptic of date, TT, f64.
//! Conversion uses the date's obliquity and inverse long-term precession to form an Earth-relative J2000
//! equatorial AU state. The coordinator composes Earth's translation at the requested epoch.
mod legacy;
mod meeus;
use crate::astro::Vector3;
pub use legacy::{MOON_ORBIT, MoonOrbit, compute_moon_age, compute_moon_geocentric, moon_age_to_phase};
pub use meeus::compute_lunar_ecliptic;

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

impl MoonPhase {
    /// All phases, from New Moon through the waxing and waning phases.
    pub const ALL: [MoonPhase; 8] = [
        MoonPhase::New,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::Full,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    /// Human readable name, e.g. "Waxing Crescent".
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 8] = [
            "New Moon",
            "Waxing Crescent",
            "First Quarter",
            "Waxing Gibbous",
            "Full Moon",
            "Waning Gibbous",
            "Last Quarter",
            "Waning Crescent",
        ];
        NAMES[self as usize]
    }
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

/// Resolve the named phase from the continuous phase angle and the waxing direction.
pub fn name_moon_phase(illumination: MoonIllumination) -> MoonPhase {
    use std::f64::consts::{PI, TAU};
    if illumination.phase_angle <= 0.03 * TAU {
        return MoonPhase::Full;
    } // phase latitude means exact i=0 is rare
    let fraction = (PI - illumination.phase_angle) / TAU;
    moon_age_to_phase(if illumination.waxing { fraction } else { 1.0 - fraction })
}

/// `moon` and `sun` are relative to the same observer at the same epoch. The observer-to-Moon vector is negated
/// to form the Moon-to-observer direction; the Sun-to-Moon separation is never confused with elongation. Waxing
/// means positive projected elongation around `reference_north` (the J2000 ecliptic pole for the Earth).
pub fn compute_moon_illumination(moon: Vector3, sun: Vector3, reference_north: Vector3) -> MoonIllumination {
    let to_observer = -moon;
    let to_sun = sun - moon;
    let cosine = (to_observer.dot(to_sun) / (to_observer.length() * to_sun.length())).clamp(-1.0, 1.0);
    MoonIllumination {
        illuminated_fraction: (1.0 + cosine) / 2.0,
        phase_angle: cosine.acos(),
        waxing: sun.cross(moon).dot(reference_north) >= 0.0,
    }
}

/// Parent-relative common-frame lunar state. Its only frame dependency is the source precession evaluated at
/// each finite-difference epoch; Earth's translation is composed at the requested epoch by the coordinator.
pub fn evaluate_moon(julian_date_tt: f64) -> super::BodyState {
    super::state::evaluate_with_velocity(julian_date_tt, |tt| {
        let (longitude, latitude, distance_km) = compute_lunar_ecliptic(tt);
        let (sl, cl) = longitude.sin_cos();
        let (sb, cb) = latitude.sin_cos();
        let native = Vector3 {
            x: cb * cl,
            y: cb * sl,
            z: sb,
        } * (distance_km / 149597870.7);
        let eps = super::orientation::compute_obliquity(tt);
        super::orientation::compute_precession_matrix(tt)
            .matrix()
            .transpose()
            .apply(super::orientation::rotate_x(-eps).apply(native))
    })
}
