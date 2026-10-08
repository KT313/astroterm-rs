//! Original table owners and bounded previews. No catalog columns are split into separate borrowed tables.
use crate::constants::TABLE_BYTE_PREVIEW_CHUNK_SIZE;
use super::{Table, TableBytes, preview_indices};
use crate::rows::{Column, Row, Preview, preview, preview_text, preview_chars, plain_column};
use crate::astro::Vector3;
use crate::cache::Cache;
use crate::canvas::Canvas;
use crate::catalog::{StarNames, cache::CatalogArray};
use crate::model::{BodySamples, CharacterStarKey, CorrectionSelection, Glyph, Moon, PixelStarKey, ProjectedMoon,
    ProjectionViewport, SelectedRegion, SelectionStats, StarKeys, StarStorage, StarRow, StarRowVec, View};
use crate::timing::StepTimes;
use bytemuck::Pod;
use image::{ImageBuffer, Pixel};
use std::{any::type_name, collections::HashMap, mem::{needs_drop, size_of}};

fn preview_slice<T: Row>(values: &[T]) -> Vec<(usize, Vec<String>)> {
    preview_indices(values.len()).map(|i| (i, values[i].cells())).collect()
}
fn nested_note<T>() -> Option<String> {
    needs_drop::<T>().then(|| "direct row payload only; nested allocations not counted".into())
}
impl<T: Row> Table for Vec<T> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn bytes(&self) -> TableBytes { TableBytes::vector(self) }
    fn columns(&self) -> Vec<Column> { T::columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { preview_slice(self) }
    fn note(&self) -> Option<String> { nested_note::<T>() }
}
impl<T: Pod + Row> Table for CatalogArray<T> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn bytes(&self) -> TableBytes { TableBytes::known(self.len() * size_of::<T>(), self.capacity() * size_of::<T>()) }
    fn columns(&self) -> Vec<Column> { T::columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { preview_slice(self) }
}

/// StarRow is a schema, not an array-of-structs allocation. Sum each owned column separately.
impl Table for StarStorage {
    fn shape(&self) -> Vec<usize> { vec![self.len(), StarRow::columns().len()] }
    fn rows(&self) -> usize { self.len() }
    fn columns(&self) -> Vec<Column> { StarRow::columns() }
    fn bytes(&self) -> TableBytes {
        let StarRowVec { u0, w, magnitude, brightness_key, distance, id, name,
            display_color } = self.owned_columns(); // adding a field requires accounting for it
        let sizes = [TableBytes::vector(u0), TableBytes::vector(w), TableBytes::vector(magnitude),
            TableBytes::vector(brightness_key), TableBytes::vector(distance),
            TableBytes::vector(id), TableBytes::vector(name),
            TableBytes::vector(display_color)];
        TableBytes {
            used: sizes.iter().try_fold(0_usize, |sum, size| sum.checked_add(size.used?)),
            reserved: sizes.iter().try_fold(0_usize, |sum, size| sum.checked_add(size.reserved?)),
        }
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        let rows = self.owned_columns();
        preview_indices(self.len()).map(|i| {
            let row = rows.get(i).expect("validated star row").to_owned();
            let mut cells = row.cells();
            cells[2] = format!("{} ({:.3} mag)", row.magnitude, crate::catalog::decode_magnitude(row.magnitude));
            cells[3] = format!("{} ({:.3} mag)", row.brightness_key, crate::catalog::decode_magnitude(row.brightness_key));
            (i, cells)
        }).collect()
    }
    fn note(&self) -> Option<String> { Some("storage=owned; per-star columns only; side tables listed separately. Magnitudes are u16 codes: code / 1000 - 10; parentheses decode the same stored value, not another column. Brightness-bound code 0 always passes early pruning.".into()) }
}

/// The live camera is one inline value owned by the root, not a cached or copied table.
impl Table for View {
    fn shape(&self) -> Vec<usize> { vec![1] }
    fn rows(&self) -> usize { 1 }
    fn bytes(&self) -> TableBytes { TableBytes::known(size_of::<Self>(), size_of::<Self>()) }
    fn columns(&self) -> Vec<Column> { plain_column::<Self>() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { vec![(0, vec![preview(self)])] }
}

/// Keep the original seven-value owner and its byte counts; only the debug preview gets named components.
pub(crate) struct PreciseMotions<'a>(pub &'a CatalogArray<[f64; 7]>);
impl Table for PreciseMotions<'_> {
    fn shape(&self) -> Vec<usize> { self.0.shape() }
    fn rows(&self) -> usize { self.0.rows() }
    fn bytes(&self) -> TableBytes { Table::bytes(self.0) }
    fn columns(&self) -> Vec<Column> {
        ["initial_direction_x", "initial_direction_y", "initial_direction_z", "scaled_velocity_x_per_year",
            "scaled_velocity_y_per_year", "scaled_velocity_z_per_year", "initial_distance_parsecs"]
            .into_iter().map(|name| Column { name, dtype: "f64" }).collect()
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.0.len()).map(|i| (i, self.0[i].iter().map(preview).collect())).collect()
    }
}

pub(crate) struct Single<'a, T>(pub &'a T);
impl<T: Row> Table for Single<'_, T> {
    fn shape(&self) -> Vec<usize> { vec![1] }
    fn rows(&self) -> usize { 1 }
    fn bytes(&self) -> TableBytes { TableBytes::known(size_of::<T>(), size_of::<T>()) }
    fn columns(&self) -> Vec<Column> { T::columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { vec![(0, self.0.cells())] }
    fn note(&self) -> Option<String> { nested_note::<T>() }
}
pub(crate) struct Opaque { pub present: bool, pub what: &'static str }
impl Table for Opaque {
    fn shape(&self) -> Vec<usize> { Vec::new() }
    fn rows(&self) -> usize { 0 }
    fn bytes(&self) -> TableBytes {
        if self.present { TableBytes { used: None, reserved: None } } else { TableBytes::known(0, 0) }
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { Vec::new() }
    fn note(&self) -> Option<String> { Some(if self.present { format!("{}; size unknown", self.what) } else { "none".into() }) }
}

/// Adapters retain the original buffer owner so allocation capacity remains available.
pub(crate) enum Bytes<'a> { Binary(&'a Vec<u8>), Text(&'a String) }
impl<'a> Bytes<'a> {
    pub fn binary(value: &'a Vec<u8>) -> Self { Self::Binary(value) }
    pub fn string(value: &'a String) -> Self { Self::Text(value) }
    fn data(&self) -> &[u8] { match self { Self::Binary(v) => v, Self::Text(v) => v.as_bytes() } }
}
fn preview_bytes(bytes: &[u8], text: bool) -> Vec<(usize, Vec<String>)> {
    preview_indices(bytes.len().div_ceil(TABLE_BYTE_PREVIEW_CHUNK_SIZE)).map(|i| {
        let boundary = |mut offset: usize| {
            if text { while offset < bytes.len() && bytes[offset] & 0xc0 == 0x80 { offset += 1; } }
            offset
        };
        let chunk = &bytes[boundary(i * TABLE_BYTE_PREVIEW_CHUNK_SIZE)..boundary(((i + 1) * TABLE_BYTE_PREVIEW_CHUNK_SIZE).min(bytes.len()))];
        (i, vec![if text { preview(String::from_utf8_lossy(chunk).as_ref()) } else { preview(chunk) }])
    }).collect()
}
impl Table for Bytes<'_> {
    fn shape(&self) -> Vec<usize> { vec![self.data().len()] }
    fn rows(&self) -> usize { self.data().len().div_ceil(TABLE_BYTE_PREVIEW_CHUNK_SIZE) }
    fn bytes(&self) -> TableBytes {
        let capacity = match self { Self::Binary(v) => v.capacity(), Self::Text(v) => v.capacity() };
        TableBytes::known(self.data().len(), capacity)
    }
    fn columns(&self) -> Vec<Column> { match self { Self::Text(_) => plain_column::<str>(), Self::Binary(_) => plain_column::<u8>() } }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { preview_bytes(self.data(), matches!(self, Self::Text(_))) }
    fn note(&self) -> Option<String> { Some(format!("rows are {TABLE_BYTE_PREVIEW_CHUNK_SIZE}-byte chunks")) }
}
impl Table for StarNames {
    fn shape(&self) -> Vec<usize> { vec![self.bytes().len()] }
    fn rows(&self) -> usize { self.bytes().len().div_ceil(TABLE_BYTE_PREVIEW_CHUNK_SIZE) }
    fn bytes(&self) -> TableBytes { TableBytes::known(self.bytes().len(), self.capacity()) }
    fn columns(&self) -> Vec<Column> { plain_column::<str>() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { preview_bytes(self.bytes(), true) }
    fn note(&self) -> Option<String> { Some(format!("rows are {TABLE_BYTE_PREVIEW_CHUNK_SIZE}-byte chunks")) }
}
pub(crate) struct TimingSteps<'a>(pub &'a StepTimes);
impl Table for TimingSteps<'_> {
    fn shape(&self) -> Vec<usize> { vec![self.0.steps().len()] }
    fn rows(&self) -> usize { self.0.steps().len() }
    fn bytes(&self) -> TableBytes { TableBytes::known(std::mem::size_of_val(self.0.steps()), self.0.step_capacity() * size_of::<crate::timing::StepTime>()) }
    fn columns(&self) -> Vec<Column> { crate::timing::StepTime::columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { preview_slice(self.0.steps()) }
}

impl<K, V: Table> Table for Cache<K, V> {
    fn shape(&self) -> Vec<usize> { self.stored().shape() }
    fn rows(&self) -> usize { self.stored().rows() }
    fn bytes(&self) -> TableBytes { self.stored().bytes() }
    fn columns(&self) -> Vec<Column> { self.stored().columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { self.stored().preview() }
    fn note(&self) -> Option<String> {
        Some(format!("{}; {}", cache_note(self), self.stored().and_then(Table::note).unwrap_or_default()))
    }
}
pub(crate) struct ScalarCache<'a, K, V>(pub &'a Cache<K, V>);
impl<K, V: Row> Table for ScalarCache<'_, K, V> {
    fn shape(&self) -> Vec<usize> { vec![self.rows()] }
    fn rows(&self) -> usize { usize::from(self.0.stored().is_some()) }
    fn bytes(&self) -> TableBytes { TableBytes::known(self.rows() * size_of::<V>(), self.rows() * size_of::<V>()) }
    fn columns(&self) -> Vec<Column> { V::columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { self.0.stored().map_or_else(Vec::new, |v| vec![(0, v.cells())]) }
    fn note(&self) -> Option<String> { Some(cache_note(self.0)) }
}
fn cache_note<K, V>(cache: &Cache<K, V>) -> String {
    let s = cache.stats;
    format!("invalid={} calculated_at={:?} valid={}s gen={} H:{} R:{} B:{}",
        cache.has_been_invalidated, cache.calculated_at, cache.valid_seconds, cache.generation, s.hits, s.refreshes, s.bypasses)
}

/// One ordering per preview. Only the edge rows invoke the formatting callback.
fn preview_map<K: Ord + Copy + std::hash::Hash, V>(map: &HashMap<K, V>, mut cells: impl FnMut(K, &V) -> Vec<String>) -> Vec<(usize, Vec<String>)> {
    let mut keys: Vec<_> = map.keys().copied().collect();
    keys.sort_unstable();
    preview_indices(keys.len()).map(|i| (i, cells(keys[i], &map[&keys[i]]))).collect()
}
/// One bounded table for the original region collection, including its nested sample allocations.
impl Table for crate::state::StellarRegions {
    fn shape(&self) -> Vec<usize> { vec![self.entries.len()] }
    fn rows(&self) -> usize { self.entries.len() }
    fn bytes(&self) -> TableBytes {
        let mut bytes = TableBytes::vector(&self.entries);
        for samples in self.entries.iter().filter_map(Cache::stored) {
            let nested = TableBytes::vector(samples);
            bytes.used = bytes.used.and_then(|n| n.checked_add(nested.used?));
            bytes.reserved = bytes.reserved.and_then(|n| n.checked_add(nested.reserved?));
        }
        bytes
    }
    fn columns(&self) -> Vec<Column> {
        vec![Column { name: "simulation_region_id", dtype: "usize" }, Column { name: "calculated_at_tt_jd", dtype: "Option<f64>" },
            Column { name: "reuse_window_simulation_seconds", dtype: "f64" }, Column { name: "has_been_invalidated", dtype: "bool" },
            Column { name: "generation", dtype: "u64" }, Column { name: "sample_count", dtype: "usize" },
            Column { name: "samples", dtype: "Vec<StellarSample>" }]
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.entries.len()).map(|region| {
            let entry = &self.entries[region];
            let samples = entry.stored().map_or_else(|| "none".into(), |samples| {
                preview_indices(samples.len()).map(|i| format!("{i}: {}", samples[i].cells().join(", "))).collect::<Vec<_>>().join("; ")
            });
            (region, vec![region.to_string(), format!("{:?}", entry.calculated_at), entry.valid_seconds.to_string(),
                entry.has_been_invalidated.to_string(), entry.generation.to_string(), entry.stored().map_or(0, Vec::len).to_string(), samples])
        }).collect()
    }
    fn note(&self) -> Option<String> {
        Some("Original region slots plus their owned sample payloads; nested capacity included, allocator overhead excluded. Final region owns constellation stars exclusively. Sample offset addresses catalog row grid.offsets[region] + offset. none differs from a valid empty vector. Previews are bounded.".into())
    }
}
impl Table for HashMap<char, Glyph> {
    fn shape(&self) -> Vec<usize> { vec![self.len()] }
    fn rows(&self) -> usize { self.len() }
    fn bytes(&self) -> TableBytes {
        TableBytes::known(self.len() * size_of::<(char, Glyph)>() + self.values().map(|g| g.coverage.len()).sum::<usize>(),
            self.capacity() * size_of::<(char, Glyph)>() + self.values().map(|g| g.coverage.capacity()).sum::<usize>())
    }
    fn columns(&self) -> Vec<Column> {
        vec![Column { name: "char", dtype: type_name::<char>() }, Column { name: "metrics", dtype: type_name::<fontdue::Metrics>() }, Column { name: "coverage_bytes", dtype: type_name::<usize>() }]
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_map(self, |key, glyph| vec![preview(&key), preview(&glyph.metrics), glyph.coverage.len().to_string()])
    }
    fn note(&self) -> Option<String> { Some("entry payload estimate plus coverage; excludes hash bucket/control overhead".into()) }
}
crate::rows::debug_preview!(fontdue::Metrics);

pub(crate) trait Describe { fn describe(&self) -> String; }
impl Describe for std::sync::Arc<crate::model::ConstellationSet> {
    fn describe(&self) -> String { format!("shared definition reference: {} figures, {} endpoints; payload counted at owner", self.figures().len(), self.endpoints().len()) }
}
macro_rules! describe_preview {
    ($($ty:ty),+) => { $(impl Describe for $ty { fn describe(&self) -> String { preview(self) } })+ };
}
describe_preview!(usize, f64, Vector3, View, ProjectionViewport, Moon, ProjectedMoon, SelectionStats);
impl<T: Preview> Describe for Vec<T> { fn describe(&self) -> String { preview(self) } }
macro_rules! vector_tuples {
    ($(($($extra:ident),+)),+) => { $(
        #[allow(non_snake_case)]
        impl<T: Row, $($extra: Describe),+> Table for (Vec<T>, $($extra),+) {
            fn shape(&self) -> Vec<usize> { self.0.shape() }
            fn rows(&self) -> usize { self.0.rows() }
            fn bytes(&self) -> TableBytes { self.0.bytes() }
            fn columns(&self) -> Vec<Column> { self.0.columns() }
            fn preview(&self) -> Vec<(usize, Vec<String>)> { self.0.preview() }
            fn note(&self) -> Option<String> {
                let (_, $($extra),+) = self;
                Some(format!("first vector payload only; auxiliary/nested allocations excluded; with {}", [$($extra.describe()),+].join(", ")))
            }
        }
    )+ };
}
vector_tuples!((A), (A, B), (A, B, C), (A, B, C, D));
macro_rules! vector_with_record {
    ($($ty:ty { $vector:ident, $record:ident }),+) => { $(
        impl Table for $ty {
            fn shape(&self) -> Vec<usize> { self.$vector.shape() }
            fn rows(&self) -> usize { self.$vector.rows() }
            fn bytes(&self) -> TableBytes { self.$vector.bytes() }
            fn columns(&self) -> Vec<Column> { self.$vector.columns() }
            fn preview(&self) -> Vec<(usize, Vec<String>)> { self.$vector.preview() }
            fn note(&self) -> Option<String> { Some(format!("vector payload only; {}={}", stringify!($record), preview(&self.$record))) }
        }
    )+ };
}
vector_with_record!(CorrectionSelection { indices, stats }, BodySamples { planets, moon }, SelectedRegion { cells, brute_force });

impl Table for StarKeys {
    fn shape(&self) -> Vec<usize> { vec![self.rows()] }
    fn rows(&self) -> usize { match self { Self::Pixels(v) => v.len(), Self::Characters { glyphs, .. } => glyphs.len() } }
    fn columns(&self) -> Vec<Column> { match self { Self::Pixels(_) => PixelStarKey::columns(), Self::Characters { .. } => CharacterStarKey::columns() } }
    fn preview(&self) -> Vec<(usize, Vec<String>)> { match self { Self::Pixels(v) => v.preview(), Self::Characters { glyphs, .. } => glyphs.preview() } }
    fn bytes(&self) -> TableBytes {
        match self {
            Self::Pixels(v) => v.bytes(),
            Self::Characters { glyphs, labels } => TableBytes::known(
                glyphs.len() * size_of::<CharacterStarKey>() + labels.len() * size_of::<(usize, String)>() + labels.iter().map(|(_, l)| l.len()).sum::<usize>(),
                glyphs.capacity() * size_of::<CharacterStarKey>() + labels.capacity() * size_of::<(usize, String)>() + labels.iter().map(|(_, l)| l.capacity()).sum::<usize>()),
        }
    }
    fn note(&self) -> Option<String> {
        Some(match self { Self::Pixels(_) => "pixel star keys".into(), Self::Characters { labels, .. } => format!("character star keys with {} labels (label bytes included)", labels.len()) })
    }
}
impl Table for Canvas {
    fn shape(&self) -> Vec<usize> { vec![self.height(), self.width()] }
    fn rows(&self) -> usize { self.height() }
    fn bytes(&self) -> TableBytes { TableBytes::known(self.height() * self.width() * size_of::<crate::canvas::Cell>(), self.allocated_cells() * size_of::<crate::canvas::Cell>()) }
    fn columns(&self) -> Vec<Column> { plain_column::<crate::canvas::Cell>() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.height()).map(|i| (i, vec![preview_chars(self.row(i).iter().filter(|c| !c.is_continuation()).map(|c| c.symbol))])).collect()
    }
}
impl<P: Pixel<Subpixel = u8>> Table for ImageBuffer<P, Vec<u8>> {
    fn shape(&self) -> Vec<usize> { vec![self.height() as usize, self.width() as usize, P::CHANNEL_COUNT as usize] }
    fn rows(&self) -> usize { self.height() as usize }
    fn bytes(&self) -> TableBytes { TableBytes::vector(self.as_raw()) }
    fn columns(&self) -> Vec<Column> { plain_column::<P>() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.height() as usize).map(|i| {
            let text = (0..self.width().min(8)).map(|x| preview(self.get_pixel(x, i as u32).channels())).collect::<Vec<_>>().join(" ");
            (i, vec![preview_text([text.as_str(), if self.width() > 8 { " …" } else { "" }])])
        }).collect()
    }
}
impl Table for ratatui::buffer::Buffer {
    fn shape(&self) -> Vec<usize> { vec![self.area.height as usize, self.area.width as usize] }
    fn rows(&self) -> usize { self.area.height as usize }
    fn bytes(&self) -> TableBytes { TableBytes::vector(&self.content) }
    fn columns(&self) -> Vec<Column> { plain_column::<ratatui::buffer::Cell>() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        let width = self.area.width as usize;
        preview_indices(self.rows()).map(|i| (i, vec![preview_text(self.content[i * width..(i + 1) * width].iter().map(|c| c.symbol()))])).collect()
    }
    fn note(&self) -> Option<String> { Some("cell vector only; external symbol strings not counted".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell as Counter, cmp::Ordering};

    thread_local! { static COMPARISONS: Counter<usize> = const { Counter::new(0) }; }
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    struct Key(usize);
    impl Ord for Key {
        fn cmp(&self, other: &Self) -> Ordering {
            COMPARISONS.with(|n| n.set(n.get() + 1));
            self.0.cmp(&other.0)
        }
    }
    impl PartialOrd for Key { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }
    #[test]
    fn map_preview_sorts_once_and_formats_only_nonoverlapping_edges() {
        for count in [0, 1, 10, 11, 20, 21, 1_000] {
            let map: HashMap<_, _> = (0..count).map(|i| (Key(i), ())).collect();
            let mut keys: Vec<_> = map.keys().copied().collect();
            COMPARISONS.with(|n| n.set(0));
            keys.sort_unstable();
            let one_sort = COMPARISONS.with(Counter::get);
            COMPARISONS.with(|n| n.set(0));
            let mut calls = 0;
            let rows = preview_map(&map, |key, _| { calls += 1; vec![key.0.to_string()] });
            assert_eq!(COMPARISONS.with(Counter::get), one_sort);
            assert_eq!(calls, count.min(20));
            assert_eq!(rows.iter().map(|(i, _)| *i).collect::<Vec<_>>(), preview_indices(count).collect::<Vec<_>>());
            for (index, values) in rows { assert_eq!(values, [index.to_string()]); }
        }
    }
    #[test]
    fn precise_motion_preview_preserves_original_owner_and_values() {
        let values = CatalogArray::from(vec![[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 7.0]]);
        let table = PreciseMotions(&values);
        assert_eq!(table.bytes(), Table::bytes(&values));
        assert_eq!(table.shape(), vec![1]);
        assert_eq!(table.columns().len(), 7);
        assert_eq!(table.preview(), vec![(0, vec!["0.1", "0.2", "0.3", "0.4", "0.5", "0.6", "7.0"].into_iter().map(String::from).collect())]);
        assert_eq!(values[0], [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 7.0]);
    }
    #[test]
    fn complete_star_table_counts_column_capacity_and_keeps_all_fields() {
        let mut catalog = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog().unwrap()).unwrap();
        let stars = &mut catalog.catalog.stars;
        stars.reserve(stars.len());
        let size = Table::bytes(stars);
        let packed_row_bytes = 2 * 12 + 2 * 2 + 4 + 4 + 4 + 1;
        assert_eq!(size.used, Some(stars.len() * packed_row_bytes));
        assert!(size.reserved.unwrap() > size.used.unwrap());
        let columns = stars.columns();
        let preview = Table::preview(stars);
        assert_eq!(preview.len(), 20);
        assert_eq!(<StarStorage as Table>::columns(stars).len(), 8);
        for (i, cells) in preview {
            assert_eq!(cells.len(), 8);
            assert_eq!(cells[0], crate::rows::preview(&columns.u0[i]));
            assert_eq!(cells[5], columns.id[i].to_string());
            assert_eq!(cells[7], columns.display_color[i].to_string());
        }
        assert_eq!(Table::bytes(stars), size); // inspection retains no data and changes no capacities
    }
    #[test]
    fn sizes_preserve_spare_capacity_and_unknown_is_not_empty() {
        let mut values = Vec::with_capacity(64);
        values.extend([1_u64, 2, 3]);
        assert_eq!(values.bytes(), TableBytes::known(24, values.capacity() * 8));
        values.clear();
        assert_eq!(values.bytes(), TableBytes::known(0, values.capacity() * 8));
        let opaque = Opaque { present: true, what: "fixture" };
        assert_eq!(opaque.bytes(), TableBytes { used: None, reserved: None });
        assert_eq!(Opaque { present: false, ..opaque }.bytes(), TableBytes::known(0, 0));
        let mut times = StepTimes::default();
        times.measure("fixture", || ());
        assert_eq!(TimingSteps(&times).bytes().reserved, Some(times.step_capacity() * size_of::<crate::timing::StepTime>()));
    }
    #[test]
    fn canvas_previews_keep_wide_glyphs_and_only_requested_rows() {
        let mut canvas = Canvas::new(50, 8);
        canvas.put_char(0, 0, '界', None);
        canvas.put_char(49, 1, 'é', None);
        let expected = canvas.to_lines();
        let rows = Table::preview(&canvas);
        assert_eq!(rows.len(), 20);
        for (i, cells) in rows { assert_eq!(cells, [expected[i].clone()]); }
        assert_eq!(Table::bytes(&canvas).reserved, Some(canvas.allocated_cells() * size_of::<crate::canvas::Cell>()));
    }
    #[test]
    fn text_chunk_boundaries_do_not_split_multibyte_characters() {
        let original = format!("{}界é{}", "a".repeat(63), "b".repeat(65));
        let rows = Bytes::string(&original).preview();
        let mut reconstructed = String::new();
        for (_, cells) in rows {
            assert!(!cells[0].contains('�'));
            reconstructed.push_str(&cells[0][1..cells[0].len() - 1]); // each short preview is a quoted string
        }
        assert_eq!(reconstructed, original);
    }

}
