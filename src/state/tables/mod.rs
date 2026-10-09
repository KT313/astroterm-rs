//! One flat listing of every data table the state holds, for memory debugging.
//!
//! Two traits do all the work:
//! - `Table`: one original container (the star table, a Vec, a cached result, an image, ...). It reports
//!   shape, allocation sizes and one bounded preview.
//! - `Tables`: something that owns tables. It visits each of them with a dotted path such as
//!   `cache.simulation.stars.regions`, descending into sub-owners.
//!
//! Every owner struct gets one listing: either a `list_tables!` line (field names plus the cache `Group` that governs
//! them) or a short hand-written `visit_tables` when a field needs a whole-owner adapter. `log.rs` walks the whole
//! tree from `ApplicationState` and prints it; see `ApplicationState::log_data`.
//!
//! Rule: a concrete type implements `Table` or `Tables`, never both, so the pass-through impls (`Option`, `Box`,
//! `&T`, `Arc`) stay unambiguous. Leaf impls for foreign and model types live in `leaves.rs`, owner listings in
//! `owners.rs`.
use crate::constants::TABLE_PREVIEW_EDGE_ROWS;
mod leaves;
mod regions;
#[cfg(feature = "memory-diagnostics")]
pub(crate) use regions::observation_nested_bytes as observation_region_bytes;
mod log;
mod labels;

pub(crate) use leaves::{Bytes, Opaque, ScalarCache, Single, TimingSteps, PreciseMotions};
use crate::cache::Group;
use crate::rows::Column;

/// Payload extent, not RSS. Unknown storage is never represented by zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableBytes {
    pub used: Option<usize>,
    pub reserved: Option<usize>,
}
impl TableBytes {
    pub(crate) fn known(used: usize, reserved: usize) -> Self { Self { used: Some(used), reserved: Some(reserved) } }
    pub(crate) fn vector<T>(values: &Vec<T>) -> Self {
        Self { used: values.len().checked_mul(std::mem::size_of::<T>()), reserved: values.capacity().checked_mul(std::mem::size_of::<T>()) }
    }
}

/// Source indices of the non-overlapping first and last rows.
pub(crate) fn preview_indices(count: usize) -> impl Iterator<Item = usize> {
    (0..count.min(TABLE_PREVIEW_EDGE_ROWS)).chain(count.min(TABLE_PREVIEW_EDGE_ROWS).max(count.saturating_sub(TABLE_PREVIEW_EDGE_ROWS))..count)
}

/// One original table. Previewing borrows its owner; only the selected rows are formatted.
pub trait Table {
    fn shape(&self) -> Vec<usize>;
    fn rows(&self) -> usize;
    fn bytes(&self) -> TableBytes;
    fn columns(&self) -> Vec<Column> { Vec::new() }
    fn note(&self) -> Option<String> { None }
    /// One preparation per dump: map implementations sort once before formatting the edge rows.
    fn preview(&self) -> Vec<(usize, Vec<String>)>;
}

/// Callback for one table: its dotted path, the table and, for cached results, the policy group.
pub type TableVisitor<'a> = dyn FnMut(&str, &dyn Table, Option<Group>) + 'a;

/// Something that owns tables: visits each with its dotted path and, for cached results, the policy group.
pub trait Tables {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>);
}

/// `prefix.name`, or just `name` at the root.
pub(crate) fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") }
}

/// Forward the original owner, including its capacity and custom preview preparation.
impl<T: Table + ?Sized> Table for &T {
    fn shape(&self) -> Vec<usize> { (**self).shape() }
    fn rows(&self) -> usize { (**self).rows() }
    fn bytes(&self) -> TableBytes { (**self).bytes() }
    fn note(&self) -> Option<String> { (**self).note() }
    fn columns(&self) -> Vec<Column> { (**self).columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { (**self).preview() }
}
impl<T: Table + ?Sized> Table for Box<T> {
    fn shape(&self) -> Vec<usize> { (**self).shape() }
    fn rows(&self) -> usize { (**self).rows() }
    fn bytes(&self) -> TableBytes { (**self).bytes() }
    fn note(&self) -> Option<String> { (**self).note() }
    fn columns(&self) -> Vec<Column> { (**self).columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { (**self).preview() }
}
/// Absent buffers keep their path and have no allocated payload.
impl<T: Table> Table for Option<T> {
    fn shape(&self) -> Vec<usize> { self.as_ref().map_or(vec![0], Table::shape) }
    fn rows(&self) -> usize { self.as_ref().map_or(0, Table::rows) }
    fn bytes(&self) -> TableBytes { self.as_ref().map_or(TableBytes::known(0, 0), Table::bytes) }
    fn note(&self) -> Option<String> { self.as_ref().map_or_else(|| Some("none".into()), Table::note) }
    fn columns(&self) -> Vec<Column> { self.as_ref().map_or_else(Vec::new, Table::columns) }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { self.as_ref().map_or_else(Vec::new, Table::preview) }
}
impl<T: Tables + ?Sized> Tables for std::sync::Arc<T> {
    fn visit_tables(&self, prefix: &str, visit: &mut TableVisitor<'_>) {
        (**self).visit_tables(prefix, visit);
    }
}

/// One listing line per owner struct.
///
/// ```ignore
/// list_tables!(Owner {
///     leaves: [plain_vec, cached_vec @ WorkingSet],   // fields that are tables; `@ Group` names the cache policy
///     scalars: [observer @ ObserverState],            // caches whose value is one record, shown as one row
///     groups: [sub_owner],                            // fields that own tables themselves
/// });
/// ```
macro_rules! list_tables {
    (@group) => { None };
    (@group $group:ident) => { Some($crate::cache::Group::$group) };
    ($owner:ty {
        leaves: [$($leaf:ident $(@ $leaf_group:ident)?),* $(,)?],
        scalars: [$($scalar:ident $(@ $scalar_group:ident)?),* $(,)?],
        groups: [$($sub:ident),* $(,)?] $(,)?
    }) => {
        impl $crate::state::Tables for $owner {
            fn visit_tables(&self, prefix: &str, visit: &mut $crate::state::TableVisitor<'_>) {
                $( visit(&$crate::state::tables::join(prefix, stringify!($leaf)), &self.$leaf, list_tables!(@group $($leaf_group)?)); )*
                $( visit(&$crate::state::tables::join(prefix, stringify!($scalar)), &$crate::state::tables::ScalarCache(&self.$scalar), list_tables!(@group $($scalar_group)?)); )*
                $( $crate::state::Tables::visit_tables(&self.$sub, &$crate::state::tables::join(prefix, stringify!($sub)), visit); )*
            }
        }
    };
}

mod owners; // declared after the macro so its listings can use `list_tables!`
