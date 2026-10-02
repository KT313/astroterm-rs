//! Human-readable notations: compass points, degrees-minutes-seconds, zodiac signs and elapsed time.

use std::fmt;

/// 16-point compass, clockwise from North.
const COMPASS_POINTS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW",
];

/// Azimuth in degrees of a 16-point compass direction such as "NNW" (case insensitive).
pub fn compass_point_to_azimuth(name: &str) -> Option<f64> {
    let index = COMPASS_POINTS
        .iter()
        .position(|point| point.eq_ignore_ascii_case(name))?;
    Some(index as f64 * 360.0 / COMPASS_POINTS.len() as f64)
}

/// Nearest 16-point compass direction of an azimuth in degrees.
pub fn azimuth_to_compass(azimuth: f64) -> &'static str {
    let sector = 360.0 / COMPASS_POINTS.len() as f64;
    let index = (azimuth.rem_euclid(360.0) / sector).round() as usize % COMPASS_POINTS.len();
    COMPASS_POINTS[index]
}

/// An angle split into degrees, minutes and seconds, displayed as `-45° 40' 44.04"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DegreesMinutesSeconds {
    pub negative: bool,
    pub degrees: u32,
    pub minutes: u32,
    pub seconds: f64,
}

impl DegreesMinutesSeconds {
    /// Split an angle in decimal degrees.
    pub fn from_degrees(angle: f64) -> DegreesMinutesSeconds {
        let magnitude = angle.abs();
        let degrees = magnitude.trunc();
        let total_minutes = (magnitude - degrees) * 60.0;
        let minutes = total_minutes.trunc();
        let seconds = (total_minutes - minutes) * 60.0;
        DegreesMinutesSeconds {
            negative: angle < 0.0,
            degrees: degrees as u32,
            minutes: minutes as u32,
            seconds,
        }
    }
}

impl fmt::Display for DegreesMinutesSeconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.negative { "-" } else { "" };
        write!(f, "{sign}{}° {}' {:.2}\"", self.degrees, self.minutes, self.seconds)
    }
}

/// The twelve signs of the tropical zodiac.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZodiacSign {
    Aries,
    Taurus,
    Gemini,
    Cancer,
    Leo,
    Virgo,
    Libra,
    Scorpio,
    Sagittarius,
    Capricorn,
    Aquarius,
    Pisces,
}

impl ZodiacSign {
    const ALL: [ZodiacSign; 12] = [
        ZodiacSign::Aries,
        ZodiacSign::Taurus,
        ZodiacSign::Gemini,
        ZodiacSign::Cancer,
        ZodiacSign::Leo,
        ZodiacSign::Virgo,
        ZodiacSign::Libra,
        ZodiacSign::Scorpio,
        ZodiacSign::Sagittarius,
        ZodiacSign::Capricorn,
        ZodiacSign::Aquarius,
        ZodiacSign::Pisces,
    ];

    /// The sign of a calendar date (month 1-12, day 1-31).
    pub fn from_date(month: u32, day: u32) -> ZodiacSign {
        const START_DAYS: [u32; 12] = [21, 20, 21, 21, 23, 23, 23, 23, 22, 22, 20, 19]; // from Aries (March) on
        let index = (month as usize + 12 - 3) % 12;
        let index = if day < START_DAYS[index] {
            (index + 11) % 12
        } else {
            index
        };
        ZodiacSign::ALL[index]
    }

    pub fn name(self) -> &'static str {
        const NAMES: [&str; 12] = [
            "Aries",
            "Taurus",
            "Gemini",
            "Cancer",
            "Leo",
            "Virgo",
            "Libra",
            "Scorpio",
            "Sagittarius",
            "Capricorn",
            "Aquarius",
            "Pisces",
        ];
        NAMES[self as usize]
    }

    pub fn symbol(self) -> char {
        const SYMBOLS: [char; 12] = ['♈', '♉', '♊', '♋', '♌', '♍', '♎', '♏', '♐', '♑', '♒', '♓'];
        SYMBOLS[self as usize]
    }
}

/// A duration in days split into years (of 365.25 days), days, hours, minutes and seconds, each truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElapsedTime {
    pub years: i64,
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
}

impl ElapsedTime {
    pub fn from_days(elapsed_days: f64) -> ElapsedTime {
        let years = (elapsed_days / 365.25).trunc();
        let remaining_days = elapsed_days - years * 365.25;
        let days = remaining_days.trunc();
        let remaining_hours = (remaining_days - days) * 24.0;
        let hours = remaining_hours.trunc();
        let remaining_minutes = (remaining_hours - hours) * 60.0;
        let minutes = remaining_minutes.trunc();
        let seconds = ((remaining_minutes - minutes) * 60.0).trunc();
        ElapsedTime {
            years: years as i64,
            days: days as i64,
            hours: hours as i64,
            minutes: minutes as i64,
            seconds: seconds as i64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_points_and_azimuths() {
        assert_eq!(compass_point_to_azimuth("NNW"), Some(337.5));
        assert_eq!(compass_point_to_azimuth("nnw"), Some(337.5));
        assert_eq!(compass_point_to_azimuth("E"), Some(90.0));
        assert_eq!(compass_point_to_azimuth("NNWW"), None);

        let cases = [
            (0.0, "N"),
            (359.0, "N"),
            (360.0, "N"),
            (334.0, "NNW"),
            (90.0, "E"),
            (200.0, "SSW"),
            (-90.0, "W"),
        ];
        for (azimuth, expected) in cases {
            assert_eq!(azimuth_to_compass(azimuth), expected, "{azimuth}");
        }
    }

    #[test]
    fn degrees_minutes_seconds() {
        assert_eq!(
            DegreesMinutesSeconds::from_degrees(123.4567).to_string(),
            "123° 27' 24.12\""
        );
        assert_eq!(
            DegreesMinutesSeconds::from_degrees(-45.6789).to_string(),
            "-45° 40' 44.04\""
        );
        assert_eq!(DegreesMinutesSeconds::from_degrees(0.0).to_string(), "0° 0' 0.00\"");
        assert_eq!(DegreesMinutesSeconds::from_degrees(-0.5).to_string(), "-0° 30' 0.00\""); // C dropped the sign
    }

    #[test]
    fn zodiac_signs_start_and_end_on_reference_dates() {
        let starts = [
            (3, 21),
            (4, 20),
            (5, 21),
            (6, 21),
            (7, 23),
            (8, 23),
            (9, 23),
            (10, 23),
            (11, 22),
            (12, 22),
            (1, 20),
            (2, 19),
        ];
        let ends = [
            (4, 19),
            (5, 20),
            (6, 20),
            (7, 22),
            (8, 22),
            (9, 22),
            (10, 22),
            (11, 21),
            (12, 21),
            (1, 19),
            (2, 18),
            (3, 20),
        ];
        for (index, sign) in ZodiacSign::ALL.iter().enumerate() {
            assert_eq!(ZodiacSign::from_date(starts[index].0, starts[index].1), *sign);
            assert_eq!(ZodiacSign::from_date(ends[index].0, ends[index].1), *sign);
        }
        assert_eq!(ZodiacSign::Sagittarius.name(), "Sagittarius");
        assert_eq!(ZodiacSign::Pisces.symbol(), '♓');
    }

    #[test]
    fn elapsed_time_components() {
        let elapsed_days = 365.25 + 30.0 + 6.0 / 24.0 + 15.0 / 1440.0 + 30.0 / 86400.0;
        let expected = ElapsedTime {
            years: 1,
            days: 30,
            hours: 6,
            minutes: 15,
            seconds: 30,
        };
        assert_eq!(ElapsedTime::from_days(elapsed_days), expected);
    }
}
