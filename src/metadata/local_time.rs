//! Observer-local civil time. Geographic boundaries are bundled; Unix zone rules come from the system IANA
//! database. Missing boundaries or rules fall back to labelled UTC, never the machine's local zone.

use crate::astro::Observer;
use chrono::{DateTime, FixedOffset, Utc};

/// A local date and time, and the name of its timezone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub time: DateTime<FixedOffset>,
    pub zone: String,
}

use crate::model::ObserverTimeZone;

pub fn resolve_observer_timezone(observer: &Observer) -> ObserverTimeZone {
    #[cfg(unix)]
    {
        use std::sync::LazyLock;
        static FINDER: LazyLock<tzf_rs::EmbeddedFinder> = LazyLock::new(tzf_rs::EmbeddedFinder::new);
        let name = FINDER.get_tz_name(observer.longitude.to_degrees(), observer.latitude.to_degrees());
        let zone = if name.is_empty() {
            None
        } else {
            tz::TimeZone::from_posix_tz(name).ok()
        };
        ObserverTimeZone { zone }
    }
    #[cfg(not(unix))]
    {
        let _ = observer;
        ObserverTimeZone {}
    }
}

pub fn convert_observer_time(state: &ObserverTimeZone, utc: DateTime<Utc>) -> LocalTime {
    #[cfg(unix)]
    if let Some((offset, zone)) = state
        .zone
        .as_ref()
        .and_then(|zone| describe_zone_at(zone, utc.timestamp()))
    {
        return LocalTime {
            time: utc.with_timezone(&offset),
            zone,
        };
    }
    LocalTime {
        time: utc.fixed_offset(),
        zone: "UTC (no timezone found)".to_string(),
    }
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

    #[test]
    fn observer_zones_follow_geography_and_season() {
        let utc = DateTime::parse_from_rfc3339("2025-07-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (latitude, longitude, zone, seconds) in
            [(35.69_f64, 139.69_f64, "JST", 32400), (52.52, 13.405, "CEST", 7200)]
        {
            let observer = Observer {
                latitude: latitude.to_radians(),
                longitude: longitude.to_radians(),
            };
            let local = convert_observer_time(&resolve_observer_timezone(&observer), utc);
            assert_eq!(local.zone, zone);
            assert_eq!(local.time.offset().local_minus_utc(), seconds);
        }
    }

    #[test]
    fn missing_zone_rules_use_explicit_utc_fallback() {
        let utc = DateTime::parse_from_rfc3339("2025-07-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let local = convert_observer_time(&ObserverTimeZone { zone: None }, utc);
        assert_eq!(local.zone, "UTC (no timezone found)");
        assert_eq!(local.time, utc.fixed_offset());
    }

    #[test]
    fn mid_ocean_uses_the_geographic_offset_or_explicit_fallback() {
        let observer = Observer {
            latitude: 0.0,
            longitude: -140_f64.to_radians(),
        };
        let utc = DateTime::parse_from_rfc3339("2025-07-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let local = convert_observer_time(&resolve_observer_timezone(&observer), utc);
        // tzf's ocean polygons assign nautical Etc/GMT zones; a missing system rule must instead be labelled UTC.
        assert!(
            local.time.offset().local_minus_utc() == -9 * 3600 || local.zone == "UTC (no timezone found)",
            "{local:?}"
        );
    }

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
