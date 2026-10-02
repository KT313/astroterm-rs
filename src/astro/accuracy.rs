//! Computational coverage and future accuracy targets, not claims about the current models.
//!
//! The computational interval is astronomical years -7974 through +12026 in TT, with an exclusive endpoint at
//! +12027-01-01. Future brightness bounds, spatial indexing and cache fingerprints must share this interval.
//! Outside it, those shortcuts must fall back to evaluating all stars. It does not bound calendar input.
//!
//! Targets compare like coordinates at the same TT; ΔT and catalog uncertainty are separate. Near means within
//! 200 Julian years of J2000, middle within 2,000, far the rest of the computational interval. Past and future must
//! be validated independently. Frame-time interpolation error must be included in the measured result.
//!
//! | Object | Coordinates | Near | Middle | Far |
//! |---|---|---|---|---|
//! | Stars | Apparent geocentric, matching reference corrections | 1″ | 1″ | 5″ |
//! | Sun/planets | Apparent topocentric, airless | 2″ | 60″ | 80″ |
//! | Moon | Apparent topocentric, airless | 15″ | 120″ | Unvalidated |
//! | Precession | Rotation vs. ERFA long-term model | 0.01″ | 0.01″ | 0.01″ |
//! | Refraction | Lift vs. the chosen Saemundsson formula | Formula agreement | Formula agreement | Formula agreement |
//!
//! No object class currently has a validated interval. Later accuracy work must establish one before the range
//! warning can claim coverage. These empty constants must not be filled merely because legacy regression tests pass.

use super::J2000;

/// Catalog proper motions use Julian years, regardless of the input calendar.
pub const JULIAN_YEAR_DAYS: f64 = 365.25;

/// A half-open interval of TT Julian dates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JulianDateInterval {
    pub start_tt: f64,
    pub end_tt: f64,
}

impl JulianDateInterval {
    /// Whether a finite TT date is inside this interval.
    pub fn contains(self, julian_date_tt: f64) -> bool {
        julian_date_tt >= self.start_tt && julian_date_tt < self.end_tt
    }
}

/// Shared future indexing domain: -7974-01-01 inclusive to +12027-01-01 exclusive, Gregorian TT.
/// Consumers: stellar brightness/motion bounds, grid queries, persistent-cache fingerprints and coverage tests.
pub const COMPUTATIONAL_INTERVAL: JulianDateInterval = JulianDateInterval {
    start_tt: -1191383.5,
    end_tt: 6113831.5,
};

/// Accuracy categories, independent of the theory used to compute them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectClass {
    Stars,
    SunAndPlanets,
    Moon,
}

/// Filled only after independent accuracy measurements; consumed by the future global coverage warning.
pub const STAR_VALIDATED_INTERVAL: Option<JulianDateInterval> = None;
/// See [`STAR_VALIDATED_INTERVAL`].
pub const PLANET_VALIDATED_INTERVAL: Option<JulianDateInterval> = None;
/// See [`STAR_VALIDATED_INTERVAL`].
pub const MOON_VALIDATED_INTERVAL: Option<JulianDateInterval> = None;

/// Future implementation target for the precession rotation, in arcseconds; current IAU 2006 is not this model.
pub const PRECESSION_TARGET_ARCSECONDS: f64 = 0.01;

/// Desired angular error in arcseconds, or `None` where no accuracy target has been assigned.
pub fn accuracy_target_arcseconds(class: ObjectClass, julian_date_tt: f64) -> Option<f64> {
    if !COMPUTATIONAL_INTERVAL.contains(julian_date_tt) {
        return None;
    }
    let years = ((julian_date_tt - J2000) / JULIAN_YEAR_DAYS).abs();
    match class {
        ObjectClass::Stars => Some(if years <= 2000.0 { 1.0 } else { 5.0 }),
        ObjectClass::SunAndPlanets => Some(if years <= 200.0 {
            2.0
        } else if years <= 2000.0 {
            60.0
        } else {
            80.0
        }),
        ObjectClass::Moon => {
            if years <= 200.0 {
                Some(15.0)
            } else if years <= 2000.0 {
                Some(120.0)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{datetime_to_julian_date, parse_utc_datetime};

    #[test]
    fn indexing_endpoints_match_the_declared_calendar() {
        let jd = |text| datetime_to_julian_date(&parse_utc_datetime(text).unwrap());
        assert_eq!(COMPUTATIONAL_INTERVAL.start_tt, jd("-7974-01-01T00:00:00"));
        assert_eq!(COMPUTATIONAL_INTERVAL.end_tt, jd("+12027-01-01T00:00:00"));
        assert!(COMPUTATIONAL_INTERVAL.contains(COMPUTATIONAL_INTERVAL.start_tt));
        assert!(COMPUTATIONAL_INTERVAL.contains(jd("+12026-12-31T12:00:00")));
        for date in [
            COMPUTATIONAL_INTERVAL.start_tt - 1.0,
            COMPUTATIONAL_INTERVAL.end_tt,
            f64::NAN,
            f64::INFINITY,
        ] {
            assert!(!COMPUTATIONAL_INTERVAL.contains(date));
        }
    }

    #[test]
    fn targets_cover_both_directions_without_claiming_validation() {
        for sign in [-1.0, 1.0] {
            for (years, planet, moon) in [
                (200.0, 2.0, Some(15.0)),
                (201.0, 60.0, Some(120.0)),
                (2001.0, 80.0, None),
            ] {
                let date = J2000 + sign * years * JULIAN_YEAR_DAYS;
                assert_eq!(
                    accuracy_target_arcseconds(ObjectClass::SunAndPlanets, date),
                    Some(planet)
                );
                assert_eq!(accuracy_target_arcseconds(ObjectClass::Moon, date), moon);
            }
        }
        assert_eq!(accuracy_target_arcseconds(ObjectClass::Stars, f64::NAN), None);
        assert_eq!(
            [
                STAR_VALIDATED_INTERVAL,
                PLANET_VALIDATED_INTERVAL,
                MOON_VALIDATED_INTERVAL
            ],
            [None; 3]
        );
    }
}
