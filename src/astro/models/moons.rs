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
