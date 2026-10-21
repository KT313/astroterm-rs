//! Integer UTC/UT calendar timestamps: uniform Gregorian days, astronomical year zero, no leap seconds.
use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use std::{io, time::{SystemTime, UNIX_EPOCH}};

pub const MILLISECONDS_PER_SECOND: i64 = 1_000;
pub const MILLISECONDS_PER_DAY: i64 = 86_400_000;
pub const UNIX_EPOCH_TIME_MS: i64 = 62_167_219_200_000;

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, message) }

/// Convert a calendar date exactly; sub-millisecond values and explicit leap seconds are unsupported.
pub fn datetime_to_time_ms(date: &NaiveDateTime) -> io::Result<i64> {
    if date.nanosecond() >= 1_000_000_000 || !date.nanosecond().is_multiple_of(1_000_000) {
        return Err(invalid("date must have millisecond precision and must not contain a leap second"));
    }
    date.and_utc().timestamp_millis().checked_add(UNIX_EPOCH_TIME_MS)
        .ok_or_else(|| invalid("calendar date exceeds the simulation timestamp range"))
}

/// Parse signed/unpadded Gregorian calendar input with an optional one-to-three-digit seconds fraction.
pub fn parse_simulation_time_ms(text: &str) -> io::Result<i64> {
    if let Some((_, fraction)) = text.rsplit_once('.')
        && (fraction.is_empty() || fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(invalid("date fraction must contain one to three decimal digits"));
    }
    let date = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|_| invalid("expected a Gregorian date yyyy-mm-ddThh:mm:ss[.sss]"))?;
    datetime_to_time_ms(&date)
}

/// Formatters can explicitly represent dates beyond chrono's range as unavailable.
pub fn time_ms_to_utc(time_ms: i64) -> Option<DateTime<Utc>> {
    let unix_ms = time_ms.checked_sub(UNIX_EPOCH_TIME_MS)?;
    DateTime::from_timestamp_millis(unix_ms)
}

/// Read the system clock without passing through floating-point Julian dates.
pub fn current_time_ms() -> io::Result<i64> { system_time_to_ms(SystemTime::now()) }

fn system_time_to_ms(time: SystemTime) -> io::Result<i64> {
    let nanoseconds = match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i128::try_from(elapsed.as_nanos()).map_err(|_| invalid("system time overflow"))?,
        Err(error) => -i128::try_from(error.duration().as_nanos()).map_err(|_| invalid("system time overflow"))?,
    };
    let timestamp = nanoseconds.div_euclid(1_000_000) + i128::from(UNIX_EPOCH_TIME_MS);
    i64::try_from(timestamp).map_err(|_| invalid("system time exceeds the simulation timestamp range"))
}

/// Parse human-facing seconds exactly, without rounding through a binary float.
pub fn parse_seconds_to_ms(text: &str) -> Result<i64, String> {
    let error = || "expected signed decimal seconds with at most three fractional digits, within i64 milliseconds".to_owned();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]), Some(b'+') => (false, &text[1..]), _ => (false, text),
    };
    let (seconds, fraction) = digits.split_once('.').map_or((digits, None), |(a, b)| (a, Some(b)));
    if seconds.is_empty() || !seconds.bytes().all(|byte| byte.is_ascii_digit()) { return Err(error()); }
    let seconds = seconds.parse::<i128>().map_err(|_| error())?;
    let fraction = match fraction {
        None => 0,
        Some(value) if !value.is_empty() && value.len() <= 3 && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<i128>().map_err(|_| error())? * 10_i128.pow(3 - value.len() as u32)
        },
        _ => return Err(error()),
    };
    let magnitude = seconds.checked_mul(1000).and_then(|value| value.checked_add(fraction)).ok_or_else(error)?;
    let signed = if negative { -magnitude } else { magnitude };
    i64::try_from(signed).map_err(|_| error())
}

/// Exact decimal seconds for labels and command arguments, including i64::MIN.
pub fn format_ms_as_seconds(milliseconds: i64) -> String {
    let magnitude = i128::from(milliseconds).abs();
    let sign = if milliseconds < 0 { "-" } else { "" };
    let mut result = format!("{sign}{}", magnitude / 1000);
    let fraction = magnitude % 1000;
    if fraction != 0 { result.push_str(format!(".{fraction:03}").trim_end_matches('0')); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::time::Duration;

    #[test]
    fn epoch_and_negative_milliseconds_round_trip_without_floats() {
        for (text, value) in [("0000-01-01T00:00:00", 0), ("0000-01-01T00:00:00.001", 1),
            ("-0001-12-31T23:59:59.999", -1), ("1970-01-01T00:00:00", UNIX_EPOCH_TIME_MS)] {
            assert_eq!(parse_simulation_time_ms(text).unwrap(), value);
            assert_eq!(datetime_to_time_ms(&time_ms_to_utc(value).unwrap().naive_utc()).unwrap(), value);
        }
        assert!(time_ms_to_utc(i64::MIN).is_none());
        assert!(time_ms_to_utc(i64::MAX).is_none());
        assert_eq!(system_time_to_ms(UNIX_EPOCH - Duration::from_nanos(1)).unwrap(), UNIX_EPOCH_TIME_MS - 1);
        assert_eq!(system_time_to_ms(UNIX_EPOCH + Duration::from_nanos(999_999)).unwrap(), UNIX_EPOCH_TIME_MS);
        assert_eq!(datetime_to_time_ms(&NaiveDate::from_ymd_opt(0, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()).unwrap(), 0);
    }

    #[test]
    fn calendar_precision_leaps_and_signed_years_are_explicit() {
        for text in ["0000-02-29T00:00:00", "2000-02-29T23:59:59.999", "1969-7-16T8:0:0", "-7974-1-1T0:0:0", "+12026-1-1T0:0:0"] {
            let value = parse_simulation_time_ms(text).unwrap();
            assert_eq!(datetime_to_time_ms(&time_ms_to_utc(value).unwrap().naive_utc()).unwrap(), value);
        }
        for text in ["1900-02-29T00:00:00", "2016-12-31T23:59:60", "2000-01-01T00:00:00.0001"] {
            assert!(parse_simulation_time_ms(text).is_err(), "{text}");
        }
    }

    #[test]
    fn decimal_seconds_are_exact_and_checked() {
        for (text, value) in [("1", 1000), ("0.001", 1), ("-0.125", -125), ("+0.010", 10),
            ("9223372036854775.807", i64::MAX), ("-9223372036854775.808", i64::MIN)] {
            assert_eq!(parse_seconds_to_ms(text).unwrap(), value);
            assert_eq!(parse_seconds_to_ms(&format_ms_as_seconds(value)).unwrap(), value);
        }
        for text in ["1e-3", "NaN", "inf", "0.0001", "9223372036854775.808", "-9223372036854775.809", "1.", ".1", ""] {
            assert!(parse_seconds_to_ms(text).is_err(), "{text}");
        }
    }
}
