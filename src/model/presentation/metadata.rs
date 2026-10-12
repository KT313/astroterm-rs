//! Stored display fields and loaded observer timezone rules.
use crate::rows::row_columns;

/// Zone rules loaded once for a fixed observer. Historical/future rules are those provided by the IANA database.
pub struct ObserverTimeZone {
    #[cfg(unix)]
    pub(crate) zone: Option<tz::TimeZone>,
}

/// One line of metadata, e.g. label "Lunar Phase" and value "Waxing Crescent".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataField {
    pub label: String,
    pub value: String,
}
row_columns!(MetadataField { label, value });


/// Display counters published by catalog installation, selection, observation and projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StarCounts {
    pub loaded: usize,
    pub candidate_regions: usize,
    pub filtered: usize,
    pub projected: usize,
}
row_columns!(StarCounts { loaded, candidate_regions, filtered, projected });
