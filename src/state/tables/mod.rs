//! One flat listing of every data table the state holds, for memory debugging.
//!
//! Two traits do all the work:
//! - `Table`: one rectangular container (a Vec, a column, a cache's stored value, an image, ...). It answers
//!   "what shape, how many bytes, and what does row `i` look like".
//! - `Tables`: something that owns tables. It visits each of them with a dotted path such as
//!   `cache.observation.motion`, descending into sub-owners.
//!
//! Every owner struct gets one listing: either a `list_tables!` line (field names plus the cache `Group` that governs
//! them) or a short hand-written `visit_tables` when a field needs a view or a wrapper. `log.rs` walks the whole
//! tree from `ApplicationState` and prints it; see `ApplicationState::log_data`.
//!
//! Rule: a concrete type implements `Table` or `Tables`, never both, so the pass-through impls (`Option`, `Box`,
//! `&T`, `Arc`) stay unambiguous. Leaf impls for foreign and model types live in `leaves.rs`, owner listings in
//! `owners.rs`.
mod leaves;
mod log;

pub(crate) use leaves::{Bytes, Named, Opaque, ScalarCache, Single};
use crate::cache::Group;
use crate::rows::Column;

/// One rectangular container: rows of one element type, or a byte buffer in fixed-size chunks.
pub trait Table {
    /// Logical extent: `[rows]`, `[rows, columns]` or `[height, width, channels]`.
    fn shape(&self) -> Vec<usize>;
    /// How many rows `row` can format; usually the first entry of `shape`.
    fn rows(&self) -> usize;
    /// Debug text of one row; the writer truncates long rows.
    fn row(&self, index: usize) -> String;
    /// Bytes holding live rows.
    fn used_bytes(&self) -> usize;
    /// Bytes allocated for this table, including the used ones.
    fn reserved_bytes(&self) -> usize;
    /// Extra facts: cache metadata, "rows own further allocations", mapped-section markers.
    fn note(&self) -> Option<String> { None }
    /// Column names and types of one row; empty when unknown (opaque payloads, caches holding nothing).
    fn columns(&self) -> Vec<Column> { Vec::new() }
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

/// Pass a borrowed table on (needed so `&&[T]`, a borrowed column, coerces to `&dyn Table`).
impl<T: Table + ?Sized> Table for &T {
    fn shape(&self) -> Vec<usize> { (**self).shape() }
    fn rows(&self) -> usize { (**self).rows() }
    fn row(&self, index: usize) -> String { (**self).row(index) }
    fn used_bytes(&self) -> usize { (**self).used_bytes() }
    fn reserved_bytes(&self) -> usize { (**self).reserved_bytes() }
    fn note(&self) -> Option<String> { (**self).note() }
    fn columns(&self) -> Vec<Column> { (**self).columns() }
}
impl<T: Table + ?Sized> Table for Box<T> {
    fn shape(&self) -> Vec<usize> { (**self).shape() }
    fn rows(&self) -> usize { (**self).rows() }
    fn row(&self, index: usize) -> String { (**self).row(index) }
    fn used_bytes(&self) -> usize { (**self).used_bytes() }
    fn reserved_bytes(&self) -> usize { (**self).reserved_bytes() }
    fn note(&self) -> Option<String> { (**self).note() }
    fn columns(&self) -> Vec<Column> { (**self).columns() }
}
/// `None` is an empty table, so optional buffers keep their path in the listing.
impl<T: Table> Table for Option<T> {
    fn shape(&self) -> Vec<usize> { self.as_ref().map_or(vec![0], Table::shape) }
    fn rows(&self) -> usize { self.as_ref().map_or(0, Table::rows) }
    fn row(&self, index: usize) -> String { self.as_ref().map_or_else(String::new, |t| t.row(index)) }
    fn used_bytes(&self) -> usize { self.as_ref().map_or(0, Table::used_bytes) }
    fn reserved_bytes(&self) -> usize { self.as_ref().map_or(0, Table::reserved_bytes) }
    fn note(&self) -> Option<String> { self.as_ref().map_or_else(|| Some("none".into()), Table::note) }
    fn columns(&self) -> Vec<Column> { self.as_ref().map_or_else(Vec::new, Table::columns) }
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
