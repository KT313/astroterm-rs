//! `Table` for the containers the state actually stores: vectors and slices, catalog arrays, N×3 views,
//! caches, hash maps, canvases, images, terminal buffers and byte buffers. Each impl formats single rows only;
//! nothing here prints a whole container.
use super::Table;
use crate::astro::Vector3;
use crate::rows::{Column, Row, plain_column};
use crate::cache::Cache;
use crate::canvas::Canvas;
use crate::catalog::{StarNames, cache::CatalogArray};
use crate::model::{
    BodySamples, CharacterStarKey, CorrectionSelection, Glyph, Moon, PixelStarKey, ProjectedMoon, ProjectionViewport,
    SelectedRegion, SelectionStats, StarKeys, View,
};
use crate::astro::models::stars::StellarSample;
use bytemuck::Pod;
use image::{ImageBuffer, Pixel};
use ndarray::ArrayView2;
use std::collections::HashMap;
use std::any::type_name;
use std::fmt::Debug;
use std::mem::{needs_drop, size_of, size_of_val};

/// One row's cells joined for display.
fn join_cells(row: &impl Row) -> String { row.cells().join(" | ") }

// --- plain rows -------------------------------------------------------------------------------------------------

impl<T: Row> Table for [T] {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn row(&self, index: usize) -> String { join_cells(&self[index]) }
    fn used_bytes(&self) -> usize { size_of_val(self) }
    fn reserved_bytes(&self) -> usize { size_of_val(self) }
    fn note(&self) -> Option<String> { nested_note::<T>() }
    fn columns(&self) -> Vec<Column> { T::columns() }
}
impl<T: Row> Table for Vec<T> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn row(&self, index: usize) -> String { join_cells(&self[index]) }
    fn used_bytes(&self) -> usize { self.len() * size_of::<T>() }
    fn reserved_bytes(&self) -> usize { self.capacity() * size_of::<T>() }
    fn note(&self) -> Option<String> { nested_note::<T>() }
    fn columns(&self) -> Vec<Column> { T::columns() }
}
/// Strings, nested vectors and the like live outside the row buffer and are not counted here.
fn nested_note<T>() -> Option<String> {
    needs_drop::<T>().then(|| "rows own further allocations, not counted".to_string())
}

impl<T: Pod + Row> Table for CatalogArray<T> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn row(&self, index: usize) -> String { join_cells(&self[index]) }
    fn used_bytes(&self) -> usize { self.len() * size_of::<T>() }
    fn reserved_bytes(&self) -> usize {
        match self {
            Self::Owned(values) => values.capacity() * size_of::<T>(),
            Self::Mapped { .. } => self.len() * size_of::<T>(),
        }
    }
    fn note(&self) -> Option<String> { self.is_mapped().then(|| "mapped section of the catalog cache file".to_string()) }
    fn columns(&self) -> Vec<Column> { T::columns() }
}

/// Vector columns (`u0`, `w`) as one N×3 table; wrap in `Named` to label the columns.
impl Table for ArrayView2<'_, f32> {
    fn shape(&self) -> Vec<usize> { ArrayView2::shape(self).to_vec() }
    fn rows(&self) -> usize { self.nrows() }
    fn row(&self, index: usize) -> String {
        self.index_axis(ndarray::Axis(0), index).iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(" | ")
    }
    fn used_bytes(&self) -> usize { self.len() * size_of::<f32>() }
    fn reserved_bytes(&self) -> usize { self.len() * size_of::<f32>() }
    fn columns(&self) -> Vec<Column> { vec![Column { name: "", dtype: type_name::<f32>() }; self.ncols()] }
}

/// The same table with its columns named position by position (for views without field names).
pub(crate) struct Named<T: Table>(pub T, pub &'static [&'static str]);
impl<T: Table> Table for Named<T> {
    fn shape(&self) -> Vec<usize> { self.0.shape() }
    fn rows(&self) -> usize { self.0.rows() }
    fn row(&self, index: usize) -> String { self.0.row(index) }
    fn used_bytes(&self) -> usize { self.0.used_bytes() }
    fn reserved_bytes(&self) -> usize { self.0.reserved_bytes() }
    fn note(&self) -> Option<String> { self.0.note() }
    fn columns(&self) -> Vec<Column> {
        self.0.columns().into_iter().enumerate().map(|(i, c)| Column { name: self.1.get(i).copied().unwrap_or(c.name), ..c }).collect()
    }
}

/// One record shown as a one-row table (the Moon, a cache's scalar value).
pub(crate) struct Single<'a, T>(pub &'a T);
impl<T: Row> Table for Single<'_, T> {
    fn shape(&self) -> Vec<usize> { vec![1] }
    fn rows(&self) -> usize { 1 }
    fn row(&self, _index: usize) -> String { join_cells(self.0) }
    fn used_bytes(&self) -> usize { size_of::<T>() }
    fn reserved_bytes(&self) -> usize { size_of::<T>() }
    fn columns(&self) -> Vec<Column> { T::columns() }
}

/// Something whose size cannot be read (an encoded terminal-protocol payload).
pub(crate) struct Opaque { pub present: bool, pub what: &'static str }
impl Table for Opaque {
    fn shape(&self) -> Vec<usize> { Vec::new() }
    fn rows(&self) -> usize { 0 }
    fn row(&self, _index: usize) -> String { String::new() }
    fn used_bytes(&self) -> usize { 0 }
    fn reserved_bytes(&self) -> usize { 0 }
    fn note(&self) -> Option<String> {
        Some(if self.present { format!("{}; size unknown", self.what) } else { "none".into() })
    }
}

// --- byte buffers ----------------------------------------------------------------------------------------------

/// A byte buffer shown in 64-byte chunks: as text when it holds text, otherwise as decimal bytes.
pub(crate) struct Bytes<'a> { bytes: &'a [u8], capacity: usize, text: bool }
const CHUNK: usize = 64;
impl<'a> Bytes<'a> {
    pub fn binary(bytes: &'a Vec<u8>) -> Self { Self { bytes, capacity: bytes.capacity(), text: false } }
    pub fn string(text: &'a String) -> Self { Self { bytes: text.as_bytes(), capacity: text.capacity(), text: true } }
    fn text(bytes: &'a [u8]) -> Self { Self { bytes, capacity: bytes.len(), text: true } }
}
impl Table for Bytes<'_> {
    fn shape(&self) -> Vec<usize> { vec![self.bytes.len()] }
    fn rows(&self) -> usize { self.bytes.len().div_ceil(CHUNK) }
    fn row(&self, index: usize) -> String {
        let chunk = &self.bytes[index * CHUNK..((index + 1) * CHUNK).min(self.bytes.len())];
        if self.text { format!("{:?}", String::from_utf8_lossy(chunk)) } else { format!("{chunk:?}") }
    }
    fn used_bytes(&self) -> usize { self.bytes.len() }
    fn reserved_bytes(&self) -> usize { self.capacity }
    fn note(&self) -> Option<String> { Some(format!("rows are {CHUNK}-byte chunks")) }
    fn columns(&self) -> Vec<Column> { if self.text { plain_column::<str>() } else { plain_column::<u8>() } }
}
/// The name string block; the per-star byte ranges are listed as `stars.name_table`.
impl Table for StarNames {
    fn shape(&self) -> Vec<usize> { Bytes::text(self.bytes()).shape() }
    fn rows(&self) -> usize { Bytes::text(self.bytes()).rows() }
    fn row(&self, index: usize) -> String { Bytes::text(self.bytes()).row(index) }
    fn used_bytes(&self) -> usize { Bytes::text(self.bytes()).used_bytes() }
    fn reserved_bytes(&self) -> usize { Bytes::text(self.bytes()).reserved_bytes() }
    fn note(&self) -> Option<String> { Bytes::text(self.bytes()).note() }
    fn columns(&self) -> Vec<Column> { Bytes::text(self.bytes()).columns() }
}

// --- caches ----------------------------------------------------------------------------------------------------

/// A cache whose value is a table: shape and rows come from the stored value, even when it is invalidated,
/// because the allocation is still held. The note carries the cache metadata; the key is never printed here.
impl<K, V: Table> Table for Cache<K, V> {
    fn shape(&self) -> Vec<usize> { self.stored().map_or(vec![0], Table::shape) }
    fn rows(&self) -> usize { self.stored().map_or(0, Table::rows) }
    fn row(&self, index: usize) -> String { self.stored().map_or_else(String::new, |v| v.row(index)) }
    fn used_bytes(&self) -> usize { self.stored().map_or(0, Table::used_bytes) }
    fn reserved_bytes(&self) -> usize { self.stored().map_or(0, Table::reserved_bytes) }
    fn note(&self) -> Option<String> {
        let inner = self.stored().and_then(Table::note).map_or_else(String::new, |n| format!("  {n}"));
        Some(format!("{}{inner}", cache_note(self)))
    }
    fn columns(&self) -> Vec<Column> { self.stored().map_or_else(Vec::new, Table::columns) }
}
/// A cache whose value is one record (observer geometry, light time, Moon lighting).
pub(crate) struct ScalarCache<'a, K, V>(pub &'a Cache<K, V>);
impl<K, V: Row> Table for ScalarCache<'_, K, V> {
    fn shape(&self) -> Vec<usize> { vec![usize::from(self.0.stored().is_some())] }
    fn rows(&self) -> usize { usize::from(self.0.stored().is_some()) }
    fn row(&self, _index: usize) -> String { join_cells(self.0.stored().expect("row within rows()")) }
    fn used_bytes(&self) -> usize { usize::from(self.0.stored().is_some()) * size_of::<V>() }
    fn reserved_bytes(&self) -> usize { self.used_bytes() }
    fn note(&self) -> Option<String> { Some(cache_note(self.0)) }
    fn columns(&self) -> Vec<Column> { V::columns() }
}
fn cache_note<K, V>(cache: &Cache<K, V>) -> String {
    let s = cache.stats;
    format!(
        "invalid={} calculated_at={:?} valid={}s gen={} H:{} R:{} B:{}",
        cache.has_been_invalidated, cache.calculated_at, cache.valid_seconds, cache.generation, s.hits, s.refreshes, s.bypasses,
    )
}

/// Per-star stellar samples; rows are sorted by catalog index so the output is stable.
impl Table for HashMap<usize, Cache<(), StellarSample>> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn row(&self, index: usize) -> String {
        let key = sorted_keys(self)[index];
        let cache = &self[&key];
        let sample = cache.stored().map_or_else(|| vec!["none".to_string(); StellarSample::columns().len()], Row::cells);
        format!("{key} | {} | {}", sample.join(" | "), cache.has_been_invalidated)
    }
    fn used_bytes(&self) -> usize { self.len() * size_of::<(usize, Cache<(), StellarSample>)>() }
    fn reserved_bytes(&self) -> usize { self.capacity() * size_of::<(usize, Cache<(), StellarSample>)>() }
    fn note(&self) -> Option<String> { Some("hash map; reserved counts bucket slots".into()) }
    fn columns(&self) -> Vec<Column> {
        let mut columns = vec![Column { name: "catalog_index", dtype: type_name::<usize>() }];
        columns.extend(StellarSample::columns());
        columns.push(Column { name: "invalid", dtype: type_name::<bool>() });
        columns
    }
}
/// Rendered glyph masks per character; rows are sorted by character.
impl Table for HashMap<char, Glyph> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn row(&self, index: usize) -> String {
        let key = sorted_keys(self)[index];
        let glyph = &self[&key];
        format!("{key:?} | {:?} | {}", glyph.metrics, glyph.coverage.len())
    }
    fn used_bytes(&self) -> usize { self.len() * size_of::<(char, Glyph)>() + self.values().map(|g| g.coverage.len()).sum::<usize>() }
    fn reserved_bytes(&self) -> usize { self.capacity() * size_of::<(char, Glyph)>() + self.values().map(|g| g.coverage.capacity()).sum::<usize>() }
    fn note(&self) -> Option<String> { Some("hash map entries plus their coverage bytes".into()) }
    fn columns(&self) -> Vec<Column> {
        vec![
            Column { name: "char", dtype: type_name::<char>() },
            Column { name: "metrics", dtype: type_name::<fontdue::Metrics>() },
            Column { name: "coverage_bytes", dtype: type_name::<usize>() },
        ]
    }
}
fn sorted_keys<K: Ord + Copy, V>(map: &HashMap<K, V>) -> Vec<K> {
    let mut keys: Vec<K> = map.keys().copied().collect();
    keys.sort_unstable();
    keys
}

// --- tuples and small composites stored in caches ----------------------------------------------------------------

/// Short text for the small values that ride along in cache tuples (counters, a view, the Moon record).
pub(crate) trait Describe { fn describe(&self) -> String; }
macro_rules! describe_by_debug {
    ($($type:ty),* $(,)?) => { $( impl Describe for $type { fn describe(&self) -> String { format!("{self:?}") } } )* };
}
describe_by_debug!(usize, f64, Vector3, View, ProjectionViewport, Moon, ProjectedMoon, SelectionStats);
/// A second vector in a tuple is summarized, never printed in full.
impl<X: Debug> Describe for Vec<X> {
    fn describe(&self) -> String { format!("[{} rows, first={:?}]", self.len(), self.first()) }
}

/// Rows come from the first vector; the other members go into the note.
macro_rules! vector_tuples {
    ($( ($($extra:ident),+) ),* $(,)?) => { $(
        #[allow(non_snake_case)]
        impl<T: Row, $($extra: Describe),+> Table for (Vec<T>, $($extra),+) {
            fn shape(&self) -> Vec<usize> { self.0.shape() }
            fn rows(&self) -> usize { self.0.rows() }
            fn row(&self, index: usize) -> String { self.0.row(index) }
            fn used_bytes(&self) -> usize { self.0.used_bytes() }
            fn reserved_bytes(&self) -> usize { self.0.reserved_bytes() }
            fn columns(&self) -> Vec<Column> { self.0.columns() }
            fn note(&self) -> Option<String> {
                let (_, $($extra),+) = self;
                let extras = [$($extra.describe()),+].join(", ");
                Some(match self.0.note() { Some(n) => format!("with {extras}; {n}"), None => format!("with {extras}") })
            }
        }
    )* };
}
vector_tuples!((A), (A, B), (A, B, C), (A, B, C, D));

/// Composites that are one vector plus a small record.
macro_rules! vector_with_record {
    ($( $type:ty { $vector:ident, $record:ident } ),* $(,)?) => { $(
        impl Table for $type {
            fn shape(&self) -> Vec<usize> { self.$vector.shape() }
            fn rows(&self) -> usize { self.$vector.rows() }
            fn row(&self, index: usize) -> String { self.$vector.row(index) }
            fn used_bytes(&self) -> usize { self.$vector.used_bytes() }
            fn reserved_bytes(&self) -> usize { self.$vector.reserved_bytes() }
            fn note(&self) -> Option<String> { Some(format!("{}={:?}", stringify!($record), self.$record)) }
            fn columns(&self) -> Vec<Column> { self.$vector.columns() }
        }
    )* };
}
vector_with_record!(
    CorrectionSelection { indices, stats },
    BodySamples { planets, moon },
    SelectedRegion { cells, brute_force },
);

/// Star keys of a scene: pixel records, or character glyphs plus their labels.
impl Table for StarKeys {
    fn shape(&self) -> Vec<usize> { vec![self.rows()] }
    fn rows(&self) -> usize {
        match self { Self::Pixels(keys) => keys.len(), Self::Characters { glyphs, .. } => glyphs.len() }
    }
    fn row(&self, index: usize) -> String {
        match self { Self::Pixels(keys) => join_cells(&keys[index]), Self::Characters { glyphs, .. } => join_cells(&glyphs[index]) }
    }
    fn columns(&self) -> Vec<Column> {
        match self { Self::Pixels(_) => PixelStarKey::columns(), Self::Characters { .. } => CharacterStarKey::columns() }
    }
    fn used_bytes(&self) -> usize {
        match self {
            Self::Pixels(keys) => keys.used_bytes(),
            Self::Characters { glyphs, labels } => glyphs.used_bytes() + labels.used_bytes() + labels.iter().map(|(_, l)| l.len()).sum::<usize>(),
        }
    }
    fn reserved_bytes(&self) -> usize {
        match self {
            Self::Pixels(keys) => keys.reserved_bytes(),
            Self::Characters { glyphs, labels } => glyphs.reserved_bytes() + labels.reserved_bytes() + labels.iter().map(|(_, l)| l.capacity()).sum::<usize>(),
        }
    }
    fn note(&self) -> Option<String> {
        match self {
            Self::Pixels(_) => Some("pixel star keys".into()),
            Self::Characters { labels, .. } => Some(format!("character star keys with {} labels (label bytes included)", labels.len())),
        }
    }
}

// --- rasters ---------------------------------------------------------------------------------------------------

/// A character canvas: one text line per row.
impl Table for Canvas {
    fn shape(&self) -> Vec<usize> { vec![self.height(), self.width()] }
    fn rows(&self) -> usize { self.height() }
    fn row(&self, index: usize) -> String { self.to_lines().into_iter().nth(index).unwrap_or_default() }
    fn used_bytes(&self) -> usize { self.height() * self.width() * size_of::<crate::canvas::Cell>() }
    fn reserved_bytes(&self) -> usize { self.used_bytes() }
    fn columns(&self) -> Vec<Column> { plain_column::<crate::canvas::Cell>() }
}

/// RGBA and RGB images: one pixel row per table row, showing the first few pixels.
impl<P: Pixel<Subpixel = u8>> Table for ImageBuffer<P, Vec<u8>> {
    fn shape(&self) -> Vec<usize> { vec![self.height() as usize, self.width() as usize, P::CHANNEL_COUNT as usize] }
    fn rows(&self) -> usize { self.height() as usize }
    fn row(&self, index: usize) -> String {
        const SHOWN: usize = 8;
        let Some(pixels) = self.rows().nth(index) else { return String::new(); };
        let mut text: Vec<String> = pixels.take(SHOWN).map(|p| format!("{:?}", p.channels())).collect();
        if self.width() as usize > SHOWN { text.push("…".into()); }
        text.join(" ")
    }
    fn used_bytes(&self) -> usize { self.as_raw().len() }
    fn reserved_bytes(&self) -> usize { self.as_raw().capacity() }
    fn columns(&self) -> Vec<Column> { plain_column::<P>() }
}

/// A ratatui cell buffer: the symbols of one terminal line per row.
impl Table for ratatui::buffer::Buffer {
    fn shape(&self) -> Vec<usize> { vec![self.area.height as usize, self.area.width as usize] }
    fn rows(&self) -> usize { self.area.height as usize }
    fn row(&self, index: usize) -> String {
        let width = self.area.width as usize;
        self.content.get(index * width..(index + 1) * width).map_or_else(String::new, |cells| cells.iter().map(|c| c.symbol()).collect())
    }
    fn used_bytes(&self) -> usize { self.content.len() * size_of::<ratatui::buffer::Cell>() }
    fn reserved_bytes(&self) -> usize { self.content.capacity() * size_of::<ratatui::buffer::Cell>() }
    fn note(&self) -> Option<String> { Some("cell vector only; symbol strings stored outside a cell are not counted".into()) }
    fn columns(&self) -> Vec<Column> { plain_column::<ratatui::buffer::Cell>() }
}
