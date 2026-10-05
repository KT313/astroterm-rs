//! Consistent logical-payload units for diagnostic event and inventory reports.
pub(crate) fn format_bytes(value: Option<usize>) -> String {
    let Some(value) = value else { return "unknown".into(); };
    let mut n = value as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit = 0;
    while n >= 1024.0 && unit < units.len() - 1 { n /= 1024.0; unit += 1; }
    if unit == 0 { format!("{value} B") } else { format!("{n:.1} {}", units[unit]) }
}
pub(crate) fn format_count(value: Option<usize>) -> String { value.map_or_else(|| "unknown".into(), |value| value.to_string()) }
