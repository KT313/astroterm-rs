//! Stored display fields and loaded observer timezone rules.

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


#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(MetadataField { label, value });
