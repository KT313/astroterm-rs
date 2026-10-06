//! Opt-in buffer enumeration contracts. Owners describe payloads; state owns collection and reporting.
use std::{collections::{BTreeMap, HashMap}, hash::BuildHasher, mem::size_of, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner { Application, Shared, External, Diagnostics }

pub use super::Quality;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Inline, Heap, Mapping, Borrowed, Alias, Unknown }

#[derive(Clone, Debug, PartialEq)]
pub struct BufferDescriptor {
    /// Field path; `sample[n]` is a snapshot-local inspection ordinal, not a persistent map-key identity.
    pub path: String,
    pub kind: Kind,
    pub owner: Owner,
    pub inline_bytes: usize,
    pub elements: Option<usize>,
    pub capacity: Option<usize>,
    pub used: Option<usize>,
    pub reserved: Option<usize>,
    pub quality: Quality,
    pub note: &'static str,
    /// Number of inspected records represented by this row (one unless children were grouped).
    pub grouped_rows: usize,
    pub unknown_sizes: usize,
    pub sum_overflowed: bool,
}

/// All traversal decisions are bounded by the collector; saved records never retain data pointers.
pub trait BufferSink {
    fn enter(&mut self, name: &str, inline_bytes: usize) -> bool;
    fn leave(&mut self);
    fn payload(&mut self, elements: usize, capacity: usize, element_bytes: usize, quality: Quality, note: &'static str);
    fn borrowed(&mut self, elements: usize, element_bytes: usize, note: &'static str);
    fn unknown(&mut self, note: &'static str);
    fn child_limit(&mut self, requested: usize) -> usize;
    fn begin_shared(&mut self, identity: usize, inline_bytes: usize) -> bool;
    fn end_shared(&mut self);
    fn mapping(&mut self, identity: usize, length: usize);
    fn set_owner(&mut self, owner: Owner) -> Owner;
}

pub trait ReportBuffers {
    /// False for elements containing no owned/shared heap payload; flat vectors are never scanned.
    const HAS_BUFFERS: bool = true;
    fn report_buffers(&self, sink: &mut dyn BufferSink);
}

pub fn report_field<T: ReportBuffers + ?Sized>(sink: &mut dyn BufferSink, name: &str, value: &T) {
    if sink.enter(name, std::mem::size_of_val(value)) {
        value.report_buffers(sink);
        sink.leave();
    }
}

pub fn report_external<T: ReportBuffers>(sink: &mut dyn BufferSink, name: &str, value: &T) {
    let previous = sink.set_owner(Owner::External);
    report_field(sink, name, value);
    sink.set_owner(previous);
}

macro_rules! flat {
    ($($ty:ty),* $(,)?) => {$(impl ReportBuffers for $ty {
        const HAS_BUFFERS: bool = false;
        fn report_buffers(&self, _: &mut dyn BufferSink) {}
    })*};
}
flat!((), bool, char, u8, u16, u32, u64, usize, i32, i64, f32, f64, std::ops::Range<usize>);

impl<T: ?Sized> ReportBuffers for &T {
    const HAS_BUFFERS: bool = false;
    fn report_buffers(&self, _: &mut dyn BufferSink) {} // borrowed storage belongs to its actual owner
}
impl<T: ReportBuffers> ReportBuffers for Option<T> {
    const HAS_BUFFERS: bool = T::HAS_BUFFERS;
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        if let Some(value) = self { report_field(sink, "Some", value); }
    }
}
impl<T: ReportBuffers, const N: usize> ReportBuffers for [T; N] {
    const HAS_BUFFERS: bool = T::HAS_BUFFERS;
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        if T::HAS_BUFFERS {
            for (i, value) in self.iter().take(sink.child_limit(N)).enumerate() {
                report_field(sink, &format!("[{i}]"), value);
            }
        }
    }
}
impl<T: ReportBuffers> ReportBuffers for Vec<T> {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        sink.payload(self.len(), self.capacity(), size_of::<T>(), Quality::ExactPayload, "element payload; nested allocations listed separately");
        if T::HAS_BUFFERS {
            for (i, value) in self.iter().take(sink.child_limit(self.len())).enumerate() {
                report_field(sink, &format!("[{i}]"), value);
            }
        }
    }
}
impl ReportBuffers for String {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        sink.payload(self.len(), self.capacity(), 1, Quality::ExactPayload, "UTF-8 bytes");
    }
}
impl<T: ReportBuffers + ?Sized> ReportBuffers for Arc<T> {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        let identity = Arc::as_ptr(self) as *const () as usize;
        if sink.begin_shared(identity, std::mem::size_of_val(self.as_ref())) {
            self.as_ref().report_buffers(sink);
            sink.end_shared();
        }
    }
}
impl<T: ReportBuffers + ?Sized> ReportBuffers for Box<T> {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        sink.payload(1, 1, std::mem::size_of_val(self.as_ref()), Quality::ExactPayload, "boxed payload; allocator overhead excluded");
        self.as_ref().report_buffers(sink);
    }
}
impl<K: ReportBuffers, V: ReportBuffers, S: BuildHasher> ReportBuffers for HashMap<K, V, S> {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        sink.payload(self.len(), self.capacity(), size_of::<(K, V)>(), Quality::LowerBound, "logical entry capacity; buckets/control bytes and allocator overhead unknown");
        if K::HAS_BUFFERS || V::HAS_BUFFERS {
            for (i, (key, value)) in self.iter().take(sink.child_limit(self.len())).enumerate() {
                report_field(sink, &format!("sample[{i}].key"), key);
                report_field(sink, &format!("sample[{i}].value"), value);
            }
        }
    }
}
impl<K: ReportBuffers, V: ReportBuffers> ReportBuffers for BTreeMap<K, V> {
    fn report_buffers(&self, sink: &mut dyn BufferSink) {
        sink.payload(self.len(), self.len(), size_of::<(K, V)>(), Quality::LowerBound, "live entries only; B-tree node capacity unknown");
        if K::HAS_BUFFERS || V::HAS_BUFFERS {
            for (i, (key, value)) in self.iter().take(sink.child_limit(self.len())).enumerate() {
                report_field(sink, &format!("sample[{i}].key"), key);
                report_field(sink, &format!("sample[{i}].value"), value);
            }
        }
    }
}
macro_rules! tuple {
    ($($ty:ident:$index:tt),+) => {
        impl<$($ty: ReportBuffers),+> ReportBuffers for ($($ty,)+) {
            const HAS_BUFFERS: bool = false $(|| $ty::HAS_BUFFERS)+;
            fn report_buffers(&self, sink: &mut dyn BufferSink) {
                $(if $ty::HAS_BUFFERS { report_field(sink, stringify!($index), &self.$index); })+
            }
        }
    };
}
tuple!(A:0); tuple!(A:0,B:1); tuple!(A:0,B:1,C:2); tuple!(A:0,B:1,C:2,D:3); tuple!(A:0,B:1,C:2,D:3,E:4);

/// Shared DTO stored on PipelineTrace; the collector implementation remains in state/memory.
#[derive(Clone, Debug, PartialEq)]
pub struct InventorySnapshot {
    pub label: &'static str,
    pub simulated_tt: Option<f64>,
    pub rows: Vec<BufferDescriptor>,
    pub omitted_nodes: usize,
    pub root_inline: usize,
    pub collector_retained_bytes: Option<usize>,
    pub collector_temporary_bytes: Option<usize>,
    pub capture_seconds: f64,
}

impl InventorySnapshot {
    pub fn used_bytes(&self) -> Option<usize> {
        self.rows.iter().try_fold(self.rows.len().checked_mul(size_of::<BufferDescriptor>())?, |sum, row| sum.checked_add(row.path.len()))
    }

    /// Payload retained by the report itself, excluding this struct's embedded headers.
    pub fn retained_bytes(&self) -> Option<usize> {
        self.rows.iter().try_fold(self.rows.capacity().checked_mul(size_of::<BufferDescriptor>())?, |sum, row| sum.checked_add(row.path.capacity()))
    }
}

macro_rules! report_fields {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl $crate::cache::ReportBuffers for $ty {
            fn report_buffers(&self, sink: &mut dyn $crate::cache::BufferSink) {
                $($crate::cache::report_field(sink, stringify!($field), &self.$field);)*
            }
        }
    };
}
pub(crate) use report_fields;
macro_rules! report_flat {
    // Tripwire for ordinary owning fields. Raw pointers/ManuallyDrop still require an ownership audit.
    ($($ty:ty),* $(,)?) => {$(const _: () = assert!(!std::mem::needs_drop::<$ty>());
    impl $crate::cache::ReportBuffers for $ty {
        const HAS_BUFFERS: bool = false;
        fn report_buffers(&self, _: &mut dyn $crate::cache::BufferSink) {}
    })*};
}
pub(crate) use report_flat;

impl ReportBuffers for image::RgbaImage {
    fn report_buffers(&self, sink: &mut dyn BufferSink) { report_field(sink, "pixels", self.as_raw()); }
}
