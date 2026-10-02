//! Julian dates and sidereal time.
//!
//! References: IERS Technical Note No. 32, and Capitaine, Wallace & Chapront, "Expressions for IAU 2000 precession
//! quantities".
//!
//! Input, display and [`SimulationClock`] use UTC, interpreted as UT1 without Earth-orientation data. Dates before
//! UTC existed (1960) use UT. This is an application approximation, not a prediction of future UTC or DUT1.
//! Ephemerides and precession take TT; currently the caller approximates TT = UT1 (ΔT = 0). ERA takes UT1, while
//! GMST takes UT1 for rotation and TT for precession. Eventually TT = UT1 + ΔT. The equation of the origins is
//! EO = ERA - GAST and already includes the equation of the equinoxes; do not add that correction twice.
//!
//! Calendar input and display use proleptic Gregorian dates and astronomical year numbering: 0 = 1 BC,
//! -1 = 2 BC. Signed years are required outside 0000–9999. No Julian-calendar switch occurs in 1582.

use std::f64::consts::{PI, TAU};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};

use super::normalize_radians;

/// The J2000.0 epoch as a Julian date.
pub const J2000: f64 = 2451545.0;

/// Julian date of the Unix epoch (1970-01-01T00:00:00 UTC).
const UNIX_EPOCH_JULIAN_DATE: f64 = 2440587.5;

const SECONDS_PER_DAY: f64 = 86400.0;

/// Simulation time that runs `speed` times faster than the wall clock (backwards for negative speeds), starting at a
/// given Julian date. It can be paused and its speed changed while running.
///
/// The time is derived from the elapsed wall-clock time since the last change, so slow frames don't make it drift.
#[derive(Clone, Copy, Debug)]
pub struct SimulationClock {
    start_julian_date: f64,
    /// Simulation time at `anchor_instant`; the clock runs on from there.
    anchor_julian_date: f64,
    anchor_instant: Instant,
    speed: f64,
    paused: bool,
}

impl SimulationClock {
    /// A running clock showing the UTC/UT date `start_julian_date_utc` now.
    pub fn start(start_julian_date_utc: f64, speed: f64) -> SimulationClock {
        SimulationClock {
            start_julian_date: start_julian_date_utc,
            anchor_julian_date: start_julian_date_utc,
            anchor_instant: Instant::now(),
            speed,
            paused: false,
        }
    }

    /// The current simulation time.
    pub fn julian_date(&self) -> f64 {
        if self.paused {
            return self.anchor_julian_date;
        }
        self.anchor_julian_date + self.anchor_instant.elapsed().as_secs_f64() / SECONDS_PER_DAY * self.speed
    }

    /// The simulation time the clock started at.
    pub fn start_julian_date(&self) -> f64 {
        self.start_julian_date
    }

    /// Simulated days per real day.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Change the speed from now on.
    pub fn set_speed(&mut self, speed: f64) {
        self.anchor_to_now();
        self.speed = speed;
    }

    /// Stop or resume the clock.
    pub fn toggle_pause(&mut self) {
        self.anchor_to_now();
        self.paused = !self.paused;
    }

    /// Continue from the current simulation time, so later changes don't affect the time that has already passed.
    fn anchor_to_now(&mut self) {
        self.anchor_julian_date = self.julian_date();
        self.anchor_instant = Instant::now();
    }
}

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

/// Julian date of the current system time.
pub fn current_julian_date() -> f64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    UNIX_EPOCH_JULIAN_DATE + since_epoch.as_secs_f64() / SECONDS_PER_DAY
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

/// Greenwich mean sidereal time in radians: rotation from UT1, mean precession from TT (Capitaine et al. eq. 42).
pub fn greenwich_mean_sidereal_time(julian_date_ut1: f64, julian_date_tt: f64) -> f64 {
    // accumulated precession in arcseconds, from Julian centuries since J2000
    let t = (julian_date_tt - J2000) / 36525.0;
    let precession_arcsec = -0.014506 - 4612.156534 * t - 1.3915817 * t.powi(2)
        + 0.00000044 * t.powi(3)
        + 0.000029956 * t.powi(4)
        + 0.0000000368 * t.powi(5);

    // subtract it from the Earth rotation angle
    let precession = precession_arcsec / 3600.0 * PI / 180.0;
    normalize_radians(earth_rotation_angle(julian_date_ut1) - precession)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 0.0001;

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

    #[test]
    fn paused_clock_stands_still_and_resumes_where_it_stopped() {
        let mut clock = SimulationClock::start(J2000, 1e6);
        clock.toggle_pause();
        let paused_at = clock.julian_date();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(clock.julian_date(), paused_at);
        assert!(clock.is_paused());

        clock.set_speed(-1e6); // changing the speed keeps the clock paused
        assert_eq!(clock.julian_date(), paused_at);
        clock.toggle_pause();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(clock.julian_date() < paused_at); // running backwards
        assert_eq!((clock.speed(), clock.start_julian_date()), (-1e6, J2000));
    }

    #[test]
    fn simulation_clock_starts_at_the_start_date() {
        let clock = SimulationClock::start(J2000, 1000.0);
        let elapsed = clock.julian_date() - J2000;
        assert!((0.0..0.01).contains(&elapsed));
    }

    #[test]
    fn current_julian_date_is_after_2025() {
        assert!(current_julian_date() > 2460676.5);
    }
}
