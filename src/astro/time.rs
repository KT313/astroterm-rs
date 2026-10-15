//! Julian dates and sidereal time.
//!
//! References: IERS Technical Note No. 32, and Capitaine, Wallace & Chapront, "Expressions for IAU 2000 precession
//! quantities".
//!
//! Input, display and [`crate::time::SimulationClock`] use UTC, interpreted as UT1 without Earth-orientation data. Dates before
//! UTC existed (1960) use UT. This is an application approximation, not a prediction of future UTC or DUT1.
//! Ephemerides, proper motion and orientation take TT = UT1 + Espenak–Meeus ΔT. ERA takes UT1.
//! GMST uses the long-term mean equator/equinox; production adds nutation to form the apparent equation of origins. The equation of the origins is
//! EO = ERA - GAST and already includes the equation of the equinoxes; do not add that correction twice.
//!
//! Calendar input and display use proleptic Gregorian dates and astronomical year numbering: 0 = 1 BC,
//! -1 = 2 BC. Signed years are required outside 0000–9999. No Julian-calendar switch occurs in 1582.

use std::f64::consts::TAU;

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};

use super::normalize_radians;

/// The J2000.0 epoch as a Julian date.
pub const J2000: f64 = 2451545.0;

/// Espenak–Meeus ΔT = TT − UT1, seconds, for a decimal Gregorian year.
/// Published polynomials: <https://eclipse.gsfc.nasa.gov/SEcat5/deltatpoly.html>.
/// The outer parabola is an extrapolation, not a prediction of Earth's future rotation.
/// No ELP-specific secular-acceleration adjustment is applied to this time-scale estimate.
pub fn estimate_delta_t(year: f64) -> f64 {
    let polynomial = |t: f64, coefficients: &[f64]| coefficients.iter().rev().fold(0.0, |v, c| v * t + c);
    match year {
        y if y < -500.0 => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2),
        y if y < 500.0 => polynomial(
            y / 100.0,
            &[
                10583.6,
                -1014.41,
                33.78311,
                -5.952053,
                -0.1798452,
                0.022174192,
                0.0090316521,
            ],
        ),
        y if y < 1600.0 => polynomial(
            (y - 1000.0) / 100.0,
            &[
                1574.2,
                -556.01,
                71.23472,
                0.319781,
                -0.8503463,
                -0.005050998,
                0.0083572073,
            ],
        ),
        y if y < 1700.0 => polynomial(y - 1600.0, &[120.0, -0.9808, -0.01532, 1.0 / 7129.0]),
        y if y < 1800.0 => polynomial(y - 1700.0, &[8.83, 0.1603, -0.0059285, 0.00013336, -1.0 / 1174000.0]),
        y if y < 1860.0 => polynomial(
            y - 1800.0,
            &[
                13.72,
                -0.332447,
                0.0068612,
                0.0041116,
                -0.00037436,
                0.0000121272,
                -0.0000001699,
                0.000000000875,
            ],
        ),
        y if y < 1900.0 => polynomial(
            y - 1860.0,
            &[7.62, 0.5737, -0.251754, 0.01680668, -0.0004473624, 1.0 / 233174.0],
        ),
        y if y < 1920.0 => polynomial(y - 1900.0, &[-2.79, 1.494119, -0.0598939, 0.0061966, -0.000197]),
        y if y < 1941.0 => polynomial(y - 1920.0, &[21.20, 0.84493, -0.076100, 0.0020936]),
        y if y < 1961.0 => polynomial(y - 1950.0, &[29.07, 0.407, -1.0 / 233.0, 1.0 / 2547.0]),
        y if y < 1986.0 => polynomial(y - 1975.0, &[45.45, 1.067, -1.0 / 260.0, -1.0 / 718.0]),
        y if y < 2005.0 => polynomial(
            y - 2000.0,
            &[63.86, 0.3345, -0.060374, 0.0017275, 0.000651814, 0.00002373599],
        ),
        y if y < 2050.0 => polynomial(y - 2000.0, &[62.92, 0.32217, 0.005589]),
        y if y < 2150.0 => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2) - 0.5628 * (2150.0 - y),
        y => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2),
    }
}

/// Temporary time scales derived at an astronomy boundary, never a stored application timestamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AstronomicalTime {
    pub utc_jd: f64,
    pub ut1_jd: f64,
    pub tt_jd: f64,
}
impl AstronomicalTime {
    pub fn from_time_ms(time_ms: i64) -> Self {
        let utc_jd = time_ms_to_utc_jd(time_ms);
        Self { utc_jd, ut1_jd: utc_jd, tt_jd: ut1_to_tt(utc_jd) }
    }
}

/// Convert the canonical calendar timestamp only where a Julian-date formula is needed.
pub fn time_ms_to_utc_jd(time_ms: i64) -> f64 {
    let days = time_ms.div_euclid(crate::time::MILLISECONDS_PER_DAY);
    let remainder_ms = time_ms.rem_euclid(crate::time::MILLISECONDS_PER_DAY);
    1_721_059.5 + days as f64 + remainder_ms as f64 / crate::time::MILLISECONDS_PER_DAY as f64
}

/// Import an external UTC/UT Julian date at the application's nearest-millisecond boundary.
/// Reference formulas that need sub-millisecond precision should keep their original floating-point date.
pub fn utc_jd_to_time_ms(utc_jd: f64) -> std::io::Result<i64> {
    let milliseconds = ((utc_jd - 1_721_059.5) * crate::time::MILLISECONDS_PER_DAY as f64).round();
    let limit = 2.0_f64.powi(63);
    if !milliseconds.is_finite() || milliseconds < -limit || milliseconds >= limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Julian date exceeds the simulation timestamp range"));
    }
    Ok(milliseconds as i64)
}

/// Convert UT1 to approximate TT using a continuous decimal Gregorian year.
pub fn ut1_to_tt(julian_date_ut1: f64) -> f64 {
    let year = julian_date_to_utc(julian_date_ut1).map_or(2000.0 + (julian_date_ut1 - J2000) / 365.2425, |date| {
        let year = date.year();
        let start = chrono::NaiveDate::from_ymd_opt(year, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let days = if start.date().leap_year() { 366.0 } else { 365.0 };
        f64::from(year) + (julian_date_ut1 - datetime_to_julian_date(&start)) / days
    });
    julian_date_ut1 + estimate_delta_t(year) / SECONDS_PER_DAY
}

/// Julian date of the Unix epoch (1970-01-01T00:00:00 UTC).
const UNIX_EPOCH_JULIAN_DATE: f64 = 2440587.5;

const SECONDS_PER_DAY: f64 = 86400.0;

/// Julian date of a proleptic Gregorian UTC/UT calendar datetime, at whole-second precision.
pub fn datetime_to_julian_date(datetime: &NaiveDateTime) -> f64 {
    // chrono's day count remains correct before -4800, where the old truncating-division formula failed
    let julian_day_number = i64::from(datetime.num_days_from_ce()) + 1721425;
    let (hour, minute, second) = (datetime.hour(), datetime.minute(), datetime.second());

    // fraction of the day relative to noon
    let day_fraction =
        (f64::from(hour) - 12.0) / 24.0 + f64::from(minute) / 1440.0 + f64::from(second) / SECONDS_PER_DAY;
    julian_day_number as f64 + day_fraction
}

/// UTC datetime of a Julian date, truncated to whole seconds. `None` outside the range chrono can represent.
pub fn julian_date_to_utc(julian_date_utc: f64) -> Option<DateTime<Utc>> {
    let unix_seconds = ((julian_date_utc - UNIX_EPOCH_JULIAN_DATE) * SECONDS_PER_DAY).floor();
    if !unix_seconds.is_finite() || unix_seconds.abs() > i64::MAX as f64 {
        return None;
    }
    DateTime::from_timestamp(unix_seconds as i64, 0)
}

/// Parse a UTC datetime in the form `yyyy-mm-ddThh:mm:ss`. Fields may omit leading zeros (e.g. `1969-7-16T8:00:00`).
pub fn parse_utc_datetime(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").ok()
}

/// Earth rotation angle from UT1, in radians, the modern replacement for Greenwich sidereal time
/// (IERS TN 32, 5.4.4 eq. 14).
pub fn earth_rotation_angle(julian_date_ut1: f64) -> f64 {
    let days_since_j2000 = julian_date_ut1 - J2000;
    let day_fraction = julian_date_ut1 - julian_date_ut1.floor();
    normalize_radians(TAU * (day_fraction + 0.7790572732640 + 0.00273781191135448 * days_since_j2000))
}

/// Greenwich mean sidereal time in radians: rotation from UT1, mean precession from TT (model-consistent long-term equation of origins).
pub fn greenwich_mean_sidereal_time(julian_date_ut1: f64, julian_date_tt: f64) -> f64 {
    normalize_radians(
        earth_rotation_angle(julian_date_ut1)
            - super::models::orientation::compute_mean_equation_of_origins(julian_date_tt),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 0.0001;

    #[test]
    fn integer_boundary_preserves_calendar_epochs_and_fractional_time_scales() {
        for text in ["-7974-01-01T00:00:00", "-0001-12-31T23:59:59", "0000-01-01T00:00:00", "2000-01-01T12:00:00", "+12026-01-01T00:00:00"] {
            let calendar = parse_utc_datetime(text).unwrap();
            let timestamp = crate::time::datetime_to_time_ms(&calendar).unwrap();
            let original = datetime_to_julian_date(&calendar);
            let dates = AstronomicalTime::from_time_ms(timestamp);
            assert_eq!(dates.utc_jd, original);
            assert_eq!(dates.ut1_jd, original);
            assert_eq!(dates.tt_jd, ut1_to_tt(original));
            let fractional = AstronomicalTime::from_time_ms(timestamp + 125);
            assert!((fractional.utc_jd - (original + 0.125 / 86400.0)).abs() < 2e-9);
            assert_eq!(utc_jd_to_time_ms(fractional.utc_jd).unwrap(), timestamp + 125);
        }
        assert!(utc_jd_to_time_ms(f64::NAN).is_err());
        assert!(utc_jd_to_time_ms(f64::INFINITY).is_err());
    }

    fn julian_date_of(text: &str) -> f64 {
        datetime_to_julian_date(&parse_utc_datetime(text).expect("valid datetime"))
    }

    #[test]
    fn datetime_to_julian_date_matches_reference() {
        assert!((julian_date_of("2000-01-01T12:00:00") - 2451545.0).abs() < EPSILON);
        assert!((julian_date_of("1999-12-31T00:00:00") - 2451543.5).abs() < EPSILON);
        assert!((julian_date_of("1969-07-20T20:17:00") - 2440423.34514).abs() < EPSILON); // Apollo 11 landing
    }

    #[test]
    fn parse_utc_datetime_accepts_padded_and_unpadded_fields() {
        let parsed = parse_utc_datetime("2025-01-01T12:34:56").expect("padded");
        assert_eq!((parsed.year(), parsed.month(), parsed.day()), (2025, 1, 1));
        assert_eq!((parsed.hour(), parsed.minute(), parsed.second()), (12, 34, 56));

        let unpadded = parse_utc_datetime("1969-7-16T8:00:00").expect("unpadded, as in the README");
        assert_eq!((unpadded.month(), unpadded.day(), unpadded.hour()), (7, 16, 8));
    }

    #[test]
    fn parse_utc_datetime_rejects_malformed_input() {
        assert!(parse_utc_datetime("2025-01-01").is_none());
        assert!(parse_utc_datetime("2025-13-01T00:00:00").is_none());
        assert!(parse_utc_datetime("yesterday").is_none());
    }

    #[test]
    fn parse_utc_datetime_is_not_shifted_by_local_time() {
        let summer = julian_date_of("2025-07-01T12:00:00"); // a DST date in many zones
        assert!((summer - 2460858.0).abs() < EPSILON);
    }

    #[test]
    fn julian_date_to_utc_inverts_datetime_to_julian_date() {
        let cases = [
            (2451545.0, "2000-01-01T12:00:00"),
            (2440587.5, "1970-01-01T00:00:00"),
            (2460678.25, "2025-01-02T18:00:00"),
        ];
        for (julian_date, expected) in cases {
            let utc = julian_date_to_utc(julian_date).expect("in range");
            assert_eq!(utc.format("%Y-%m-%dT%H:%M:%S").to_string(), expected);
        }
        assert!(julian_date_to_utc(f64::INFINITY).is_none());
        assert!(julian_date_to_utc(1e15).is_none());
    }

    #[test]
    fn greenwich_mean_sidereal_time_matches_reference() {
        assert!((greenwich_mean_sidereal_time(J2000, J2000) - 4.89496121282306).abs() < EPSILON);
    }

    #[test]
    fn gregorian_calendar_round_trips_across_the_computational_interval() {
        for (year, month, day, hour) in [
            (-7974, 1, 1, 0),
            (-4801, 1, 1, 0),
            (-4800, 2, 29, 12),
            (-1, 1, 1, 0),
            (0, 2, 29, 12),
            (1, 1, 1, 0),
            (1582, 10, 15, 0),
            (2000, 1, 1, 12),
            (12026, 12, 31, 0),
            (12027, 1, 1, 0),
        ] {
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)
                .unwrap()
                .and_hms_opt(hour, 0, 0)
                .unwrap();
            let text = date.format("%Y-%m-%dT%H:%M:%S").to_string();
            assert_eq!(parse_utc_datetime(&text), Some(date), "{text}");
            assert_eq!(
                julian_date_to_utc(datetime_to_julian_date(&date)).unwrap().naive_utc(),
                date,
                "{text}"
            );
        }
        assert_eq!(julian_date_of("-4713-11-24T12:00:00"), 0.0);
        assert_eq!(julian_date_of("2000-01-01T12:00:00"), J2000);
        assert_eq!(
            julian_date_of("1582-10-15T00:00:00") - julian_date_of("1582-10-04T00:00:00"),
            11.0
        );
    }

    #[test]
    fn sidereal_rotation_and_precession_use_separate_time_scales() {
        let (ut1, tt) = (J2000 + 100.0, J2000 + 100.001);
        let changed_tt = greenwich_mean_sidereal_time(ut1, tt) - greenwich_mean_sidereal_time(ut1, ut1);
        assert!(changed_tt > 0.0 && changed_tt < 1e-8);
        let changed_ut = greenwich_mean_sidereal_time(ut1 + 0.001, tt) - greenwich_mean_sidereal_time(ut1, tt);
        assert!((changed_ut - 0.00630038748675).abs() < 1e-8);
    }




}

#[cfg(test)]
mod delta_t_tests {
    use super::*;
    #[test]
    fn published_polynomials_at_named_epochs() {
        for (year, expected) in [
            (1900.0, -2.79),
            (2000.0, 63.86),
            (2026.0, 75.074584),
            (4026.0, 15552.5952),
        ] {
            assert!(
                (estimate_delta_t(year) - expected).abs() < 1e-6,
                "{year}: {}",
                estimate_delta_t(year)
            );
        }
    }
    #[test]
    fn each_polynomial_boundary_is_finite_and_tt_uses_delta_t() {
        for year in [
            -500.0, 500.0, 1600.0, 1700.0, 1800.0, 1860.0, 1900.0, 1920.0, 1941.0, 1961.0, 1986.0, 2005.0, 2050.0,
            2150.0,
        ] {
            for offset in [-0.000001, 0.0, 0.000001] {
                assert!(estimate_delta_t(year + offset).is_finite());
            }
        }
        assert!(((ut1_to_tt(J2000) - J2000) * 86400.0 - 63.86).abs() < 0.002);
        assert!(
            ut1_to_tt(crate::astro::COMPUTATIONAL_INTERVAL.end_tt) > crate::astro::COMPUTATIONAL_INTERVAL.end_tt + 3.0
        );
    }
}
