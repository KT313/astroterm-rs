//! Bounded collection of typed storage inventories. No interpretation of debugger/allocator layouts.
use crate::constants::{INVENTORY_DETAIL_CHILDREN, INVENTORY_MAX_CHILDREN, INVENTORY_MAX_DEPTH, INVENTORY_MAX_ROWS, INVENTORY_MAX_VISITS};
use std::mem::size_of;
use crate::cache::{BufferDescriptor, BufferSink, InventorySnapshot, Kind, Owner, Quality};


pub struct InventoryCollector {
    snapshot: InventorySnapshot,
    path: String,
    parents: Vec<usize>,
    owner: Owner,
    shared_owners: Vec<Owner>,
    seen: Vec<(Kind, usize)>,
    visits: usize,
}
/// Keep a few child examples, then aggregate like fields of the remaining inspected children.
fn group_child_name(name: &str) -> Option<(&str, &str)> {
    let start = if name.starts_with('[') { 0 } else if name.starts_with("sample[") { 6 } else { return None; };
    let end = name[start..].find(']')? + start;
    let index = name[start + 1..end].parse::<usize>().ok()?;
    (index >= INVENTORY_DETAIL_CHILDREN).then_some((&name[..start], &name[end + 1..]))
}

fn merge_count(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    a.zip(b).and_then(|(a, b)| a.checked_add(b))
}
fn merge_bytes(a: Option<usize>, b: Option<usize>, overflowed: &mut bool) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => { let total = a.checked_add(b); *overflowed |= total.is_none(); total },
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub struct KnownPayload {
    pub used: Option<usize>,
    pub reserved: Option<usize>,
    pub unknown_records: usize,
}
/// Unknown contributions do not erase known amounts; None totals indicate arithmetic overflow only.
pub fn sum_known_payload(snapshot: &InventorySnapshot, owner: Owner) -> KnownPayload {
    sum_payload(snapshot.rows.iter().filter(|r| r.owner == owner))
}

pub(super) fn sum_payload<'a>(rows: impl Iterator<Item = &'a BufferDescriptor>) -> KnownPayload {
    let mut total = KnownPayload { used: Some(0), reserved: Some(0), unknown_records: 0 };
    for row in rows.filter(|r| matches!(r.kind, Kind::Heap | Kind::Unknown)) {
        total.unknown_records = total.unknown_records.saturating_add(row.unknown_sizes);
        if row.sum_overflowed { total.used = None; total.reserved = None; }
        if let Some(value) = row.used { total.used = total.used.and_then(|n| n.checked_add(value)); }
        if let Some(value) = row.reserved { total.reserved = total.reserved.and_then(|n| n.checked_add(value)); }
    }
    total
}

impl InventoryCollector {
    pub fn new(label: &'static str, simulated_tt: Option<f64>) -> Self {
        Self {
            snapshot: InventorySnapshot { label, simulated_tt, rows: Vec::new(), omitted_nodes: 0, root_inline: 0, collector_retained_bytes: None, collector_temporary_bytes: None, capture_seconds: 0.0 },
            path: String::new(), parents: Vec::new(), owner: Owner::Application, shared_owners: Vec::new(), seen: Vec::new(), visits: 0,
        }
    }
    pub fn finish(mut self) -> InventorySnapshot {
        self.snapshot.collector_retained_bytes = self.snapshot.retained_bytes();
        self.snapshot.collector_temporary_bytes = self.path.capacity()
            .checked_add(self.parents.capacity().saturating_mul(size_of::<usize>()))
            .and_then(|n| n.checked_add(self.shared_owners.capacity().saturating_mul(size_of::<Owner>())))
            .and_then(|n| n.checked_add(self.seen.capacity().saturating_mul(size_of::<(Kind, usize)>())));
        self.snapshot
    }
    fn append(&mut self, kind: Kind, inline_bytes: usize, counts: Option<(usize, usize)>, sizes: (Option<usize>, Option<usize>), quality: Quality, note: &'static str) {
        let unknown_sizes = usize::from(kind == Kind::Unknown || (kind == Kind::Heap && (sizes.0.is_none() || sizes.1.is_none())));
        if self.path.contains("[*]")
            && let Some(row) = self.snapshot.rows.iter_mut().find(|row| row.path == self.path && row.kind == kind && row.owner == self.owner && row.note == note) {
            row.grouped_rows = row.grouped_rows.saturating_add(1);
            row.unknown_sizes = row.unknown_sizes.saturating_add(unknown_sizes);
            row.elements = merge_count(row.elements, counts.map(|n| n.0));
            row.capacity = merge_count(row.capacity, counts.map(|n| n.1));
            if !row.sum_overflowed {
                row.used = merge_bytes(row.used, sizes.0, &mut row.sum_overflowed);
                row.reserved = merge_bytes(row.reserved, sizes.1, &mut row.sum_overflowed);
            }
            if row.sum_overflowed { row.used = None; row.reserved = None; }
            row.quality = match (row.quality, quality) {
                (Quality::Unknown, _) | (_, Quality::Unknown) => Quality::Unknown,
                (Quality::LowerBound, _) | (_, Quality::LowerBound) => Quality::LowerBound,
                _ => Quality::ExactPayload,
            };
            return;
        }
        if self.snapshot.rows.len() == INVENTORY_MAX_ROWS {
            self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(1);
            return;
        }
        self.snapshot.rows.push(BufferDescriptor { path: self.path.clone(), kind, owner: self.owner, inline_bytes,
            elements: counts.map(|v| v.0), capacity: counts.map(|v| v.1), used: sizes.0, reserved: sizes.1, quality, note,
            grouped_rows: 1, unknown_sizes, sum_overflowed: false });
    }
    fn first_allocation(&mut self, kind: Kind, identity: usize) -> bool {
        if self.seen.contains(&(kind, identity)) { return false; }
        self.seen.push((kind, identity));
        true
    }
}
impl BufferSink for InventoryCollector {
    fn enter(&mut self, name: &str, inline_bytes: usize) -> bool {
        if self.parents.len() >= INVENTORY_MAX_DEPTH || self.snapshot.rows.len() >= INVENTORY_MAX_ROWS || self.visits >= INVENTORY_MAX_VISITS {
            self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(1);
            return false;
        }
        self.visits += 1;
        if self.parents.is_empty() && self.snapshot.rows.is_empty() { self.snapshot.root_inline = inline_bytes; }
        self.parents.push(self.path.len());
        if !self.path.is_empty() { self.path.push('.'); }
        if let Some((prefix, suffix)) = group_child_name(name) {
            self.path.push_str(prefix);
            self.path.push_str("[*]");
            self.path.extend(suffix.chars().take(128));
        } else {
            self.path.extend(name.chars().take(128)); // caller-defined labels cannot grow a path without bound
        }
        if name.chars().take(129).count() > 128 { self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(1); }
        self.append(Kind::Inline, inline_bytes, None, (None, None), Quality::ExactPayload, "embedded header; not added again to aggregate payload");
        true
    }
    fn leave(&mut self) { self.path.truncate(self.parents.pop().expect("balanced inventory scopes")); }
    fn payload(&mut self, elements: usize, capacity: usize, element_bytes: usize, quality: Quality, note: &'static str) {
        let sizes = (elements.checked_mul(element_bytes), capacity.checked_mul(element_bytes));
        self.append(Kind::Heap, 0, Some((elements, capacity)), sizes,
            if sizes.0.is_none() || sizes.1.is_none() { Quality::Unknown } else { quality }, note);
    }
    fn borrowed(&mut self, elements: usize, element_bytes: usize, note: &'static str) {
        let bytes = elements.checked_mul(element_bytes);
        self.append(Kind::Borrowed, 0, Some((elements, elements)), (bytes, None), if bytes.is_some() { Quality::ExactPayload } else { Quality::Unknown }, note);
    }
    fn unknown(&mut self, note: &'static str) { self.append(Kind::Unknown, 0, None, (None, None), Quality::Unknown, note); }
    fn child_limit(&mut self, requested: usize) -> usize {
        let allowed = requested.min(INVENTORY_MAX_CHILDREN).min(INVENTORY_MAX_VISITS.saturating_sub(self.visits)).min(INVENTORY_MAX_ROWS.saturating_sub(self.snapshot.rows.len()));
        self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(requested - allowed);
        allowed
    }
    fn begin_shared(&mut self, identity: usize, inline_bytes: usize) -> bool {
        if self.snapshot.rows.len() >= INVENTORY_MAX_ROWS { self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(1); return false; }
        if !self.first_allocation(Kind::Heap, identity) {
            self.append(Kind::Alias, 0, None, (None, None), Quality::ExactPayload, "shared allocation already counted in this snapshot");
            return false;
        }
        self.shared_owners.push(self.owner);
        self.owner = Owner::Shared;
        self.payload(1, 1, inline_bytes, Quality::LowerBound, "shared payload header; Arc control block and allocator overhead unknown");
        true
    }
    fn end_shared(&mut self) { self.owner = self.shared_owners.pop().expect("balanced shared scopes"); }
    fn mapping(&mut self, identity: usize, length: usize) {
        if self.snapshot.rows.len() >= INVENTORY_MAX_ROWS { self.snapshot.omitted_nodes = self.snapshot.omitted_nodes.saturating_add(1); return; }
        if self.first_allocation(Kind::Mapping, identity) {
            self.append(Kind::Mapping, 0, None, (Some(length), Some(length)), Quality::ExactPayload, "logical mapping length, not heap or resident pages");
        } else {
            self.append(Kind::Alias, 0, None, (None, None), Quality::ExactPayload, "mapping already counted in this snapshot");
        }
    }
    fn set_owner(&mut self, owner: Owner) -> Owner { std::mem::replace(&mut self.owner, owner) }
}

