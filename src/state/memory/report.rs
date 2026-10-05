//! Human-readable views of captured inventory records. No live application data is accessed here.
use std::io::{self, Write};
use crate::timing::formatting::{format_bytes, format_count};
use crate::cache::buffers::{BufferDescriptor, InventorySnapshot, Kind, Owner, Quality};
use super::{sum_known_payload, sum_payload, MAX_ROWS, MAX_DEPTH, MAX_CHILDREN, DETAIL_CHILDREN, MAX_VISITS};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group { Catalog, Simulation, Observation, Projection, Rendering, Configuration, Diagnostics, External, Other }
impl Group {
    fn label(self) -> &'static str {
        match self {
            Self::Catalog => "Catalog — immutable shared source data",
            Self::Simulation => "Simulation — saved model samples",
            Self::Observation => "Observation — selected stars and calculated sky",
            Self::Projection => "Projection — screen positions and drawing order",
            Self::Rendering => "Rendering — canvases, text and output buffers",
            Self::Configuration => "Configuration",
            Self::Diagnostics => "Diagnostics — retained report data",
            Self::External => "External — scoped session and incomplete coverage",
            Self::Other => "Other captured data",
        }
    }
}

fn belongs_to(path: &str, prefix: &str) -> bool {
    path == prefix || path.strip_prefix(prefix).is_some_and(|tail| tail.starts_with('.'))
}
fn classify_group(row: &BufferDescriptor) -> Group {
    if row.owner == Owner::Diagnostics { return Group::Diagnostics; }
    if row.owner == Owner::External { return Group::External; }
    let path = row.path.as_str();
    if belongs_to(path, "state.catalog") { return Group::Catalog; }
    if belongs_to(path, "state.run.simulation") { return Group::Simulation; }
    if belongs_to(path, "state.run.sky") || belongs_to(path, "state.run.observation") { return Group::Observation; }
    if belongs_to(path, "state.run.projection") { return Group::Projection; }
    if belongs_to(path, "state.run.rendering") { return Group::Rendering; }
    if belongs_to(path, "state.config") { return Group::Configuration; }
    Group::Other
}

fn format_total(value: Option<usize>) -> String { value.map_or_else(|| "overflow".into(), |n| format_bytes(Some(n))) }
fn sum_mappings<'a>(rows: impl Iterator<Item = &'a BufferDescriptor>) -> Option<usize> {
    rows.filter(|r| r.kind == Kind::Mapping).try_fold(0_usize, |sum, row| sum.checked_add(row.used?))
}

fn write_row(row: &BufferDescriptor, output: &mut impl Write) -> io::Result<()> {
    write!(output, "  {} [{:?}/{:?}] ", row.path, row.owner, row.kind)?;
    match row.kind {
        Kind::Alias => write!(output, "reference only; adds no payload to totals")?,
        Kind::Mapping => write!(output, "logical length {} (not resident memory)", format_bytes(row.used))?,
        Kind::Unknown => write!(output, "size unknown; excluded from known totals")?,
        Kind::Borrowed => write!(output, "len={}; referenced {}; adds no owned payload", format_count(row.elements), format_bytes(row.used))?,
        _ => write!(output, "len={} capacity={}; used {}; reserved {}", format_count(row.elements), format_count(row.capacity), format_bytes(row.used), format_bytes(row.reserved))?,
    }
    let quality = match row.quality { Quality::ExactPayload => "exact payload", Quality::LowerBound => "partial/lower bound", Quality::Unknown => "partial/unknown" };
    writeln!(output, "; grouped={}; unknown sizes={}; {quality}: {}", row.grouped_rows, row.unknown_sizes, row.note)
}

fn write_group(snapshot: &InventorySnapshot, group: Group, output: &mut impl Write) -> io::Result<()> {
    let rows = || snapshot.rows.iter().filter(|row| row.kind != Kind::Inline && classify_group(row) == group);
    if rows().next().is_none() { return Ok(()); }
    writeln!(output, "\n{}:", group.label())?;
    for owner in [Owner::Application, Owner::Shared, Owner::Diagnostics, Owner::External] {
        if !rows().any(|r| r.owner == owner && matches!(r.kind, Kind::Heap | Kind::Unknown)) { continue; }
        let total = sum_payload(rows().filter(|r| r.owner == owner));
        writeln!(output, "  Known {owner:?} payload: used {}; reserved {}; unknown records={}", format_total(total.used), format_total(total.reserved), total.unknown_records)?;
    }
    if rows().any(|r| r.kind == Kind::Mapping) { writeln!(output, "  Mapped logical bytes: {}", format_bytes(sum_mappings(rows())))?; }
    for row in rows() { write_row(row, output)?; }
    Ok(())
}

/// Format captured values only. The terminal/session and working storage may already have been dropped.
pub fn write_inventory(snapshot: &InventorySnapshot, output: &mut impl Write) -> io::Result<()> {
    let tt = snapshot.simulated_tt.map_or_else(|| "not recorded".into(), |value| format!("{value:.9}"));
    writeln!(output, "\nMemory inventory: {} (captured before terminal cleanup; TT Julian date={tt})", snapshot.label)?;
    writeln!(output, "Payload only: reserved includes used; do not add them. Shared allocations and mapped files count once per snapshot. Mapping length is not RSS. Aliases/borrowed rows add no owned payload.")?;
    writeln!(output, "Group subtotals partition the same owner totals below; do not add both. Shared payload belongs to the first inspected path; later paths are references, not another allocation.")?;
    writeln!(output, "Root inline: {}. Inspection limits: {MAX_ROWS} rows, {MAX_DEPTH} levels, {MAX_CHILDREN} nested entries per container, {MAX_VISITS} visits; omitted nodes={}", format_bytes(Some(snapshot.root_inline)), snapshot.omitted_nodes)?;
    let partial = snapshot.rows.iter().filter(|r| r.quality != Quality::ExactPayload).fold(0_usize, |n, row| n.saturating_add(row.grouped_rows));
    writeln!(output, "Payload coverage: {}; estimated/unknown records={partial}. Unknown contributions do not erase known amounts; omitted children are not extrapolated.", if partial > 0 || snapshot.omitted_nodes > 0 { "partial" } else { "known payload only (allocator/OS overhead excluded)" })?;
    for owner in [Owner::Application, Owner::Shared, Owner::External, Owner::Diagnostics] {
        let total = sum_known_payload(snapshot, owner);
        writeln!(output, "Known {owner:?} payload: used {}; reserved {}; unknown records={} (known amounts only)", format_total(total.used), format_total(total.reserved), total.unknown_records)?;
    }
    writeln!(output, "Mapped logical bytes: {}. Collector retained payload: {}; temporary scratch: {}; capture: {:.3} ms", format_bytes(sum_mappings(snapshot.rows.iter())), format_bytes(snapshot.collector_retained_bytes), format_bytes(snapshot.collector_temporary_bytes), snapshot.capture_seconds * 1000.0)?;
    writeln!(output, "Collector figures describe this capture; diagnostic rows describe history already retained when captured. They overlap and must not be summed blindly. Report formatting/output happen later and are excluded from capture time.")?;
    writeln!(output, "Retained scratch may have len=0 and nonzero capacity after clear. Pixel image/ratatui results remain inspectable but are rebuilt per frame; state ownership does not promise allocation reuse.")?;
    writeln!(output, "sample[n] labels are snapshot-local ordinals. [*] sums inspected children after the first {DETAIL_CHILDREN} examples. Small local buffers, allocator metadata and opaque library internals remain partial.")?;
    for group in [Group::Catalog, Group::Simulation, Group::Observation, Group::Projection, Group::Rendering, Group::Configuration, Group::Diagnostics, Group::External, Group::Other] {
        write_group(snapshot, group, output)?;
    }
    Ok(())
}
