//! Converting the simulation time to the observer's local time, labelled with the timezone abbreviation.
//!
//! On Unix, the system timezone database (`TZ`, `/etc/localtime`) provides the abbreviation in effect at the given
//! time, e.g. "CET" or "CEST". Elsewhere, or if the database is unavailable, the UTC offset is shown instead.

use chrono::{DateTime, FixedOffset, Local, Utc};

/// A local date and time, and the name of its timezone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub time: DateTime<FixedOffset>,
    /// Abbreviation such as "JST", or the UTC offset such as "+09:00".
    pub zone: String,
}

/// The local time at `utc`.
pub fn convert_to_local_time(utc: DateTime<Utc>) -> LocalTime {
    if let Some((offset, zone)) = find_system_zone(utc.timestamp()) {
        return LocalTime {
            time: utc.with_timezone(&offset),
            zone,
        };
    }
    let time = utc.with_timezone(&Local).fixed_offset();
    LocalTime {
        time,
        zone: time.offset().to_string(),
    }
}

/// UTC offset and abbreviation of the system timezone at `unix_time`. The timezone is loaded once.
#[cfg(unix)]
fn find_system_zone(unix_time: i64) -> Option<(FixedOffset, String)> {
    use std::sync::OnceLock;

    static SYSTEM_ZONE: OnceLock<Option<tz::TimeZone>> = OnceLock::new();
    let zone = SYSTEM_ZONE.get_or_init(load_system_zone).as_ref()?;
    describe_zone_at(zone, unix_time)
}

/// The timezone named by `TZ` (a zone name such as `Europe/Berlin`, a file, or a POSIX rule), or `/etc/localtime`
/// if `TZ` is unset or empty, as the C library resolves it.
#[cfg(unix)]
fn load_system_zone() -> Option<tz::TimeZone> {
    match std::env::var("TZ") {
        Ok(tz_string) if !tz_string.is_empty() => tz::TimeZone::from_posix_tz(&tz_string).ok(),
        _ => tz::TimeZone::local().ok(),
    }
}

#[cfg(not(unix))]
fn find_system_zone(_unix_time: i64) -> Option<(FixedOffset, String)> {
    None
}

/// UTC offset and abbreviation of `zone` at `unix_time`, falling back to the offset if the zone has no abbreviation.
#[cfg(unix)]
fn describe_zone_at(zone: &tz::TimeZone, unix_time: i64) -> Option<(FixedOffset, String)> {
    let local_time_type = zone.find_local_time_type(unix_time).ok()?;
    let offset = FixedOffset::east_opt(local_time_type.ut_offset())?;
    let abbreviation = local_time_type.time_zone_designation();
    let name = if abbreviation.is_empty() {
        offset.to_string()
    } else {
        abbreviation.to_string()
    };
    Some((offset, name))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn unix_time(rfc3339: &str) -> i64 {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp()
    }

    #[test]
    fn names_the_zone_in_effect_at_the_time() {
        let central_europe = tz::TimeZone::from_posix_tz("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        let winter = describe_zone_at(&central_europe, unix_time("2025-01-02T12:00:00Z")).unwrap();
        let summer = describe_zone_at(&central_europe, unix_time("2025-07-02T12:00:00Z")).unwrap();
        assert_eq!(winter, (FixedOffset::east_opt(3600).unwrap(), "CET".to_string()));
        assert_eq!(summer, (FixedOffset::east_opt(7200).unwrap(), "CEST".to_string()));
    }

    #[test]
    fn converts_utc_to_local_time() {
        let japan = tz::TimeZone::from_posix_tz("JST-9").unwrap();
        let (offset, zone) = describe_zone_at(&japan, unix_time("2025-01-02T12:00:00Z")).unwrap();
        let utc = DateTime::parse_from_rfc3339("2025-01-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(utc.with_timezone(&offset).to_rfc3339(), "2025-01-02T21:00:00+09:00");
        assert_eq!(zone, "JST");
    }
}
