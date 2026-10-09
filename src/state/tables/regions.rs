//! One table per original regional owner; only bounded previews visit sample contents.
use super::{Table, TableBytes, preview_indices};
use crate::{cache::Cache, rows::{Column, Preview, preview}};
#[cfg(test)] use std::mem::size_of;

pub(super) struct RegionalTable<'a, K, V> {
    pub entries: &'a Vec<Cache<K, V>>,
    pub nested_bytes: fn(&V) -> TableBytes,
}
impl<K: Preview, V: Preview> Table for RegionalTable<'_, K, V> {
    fn shape(&self) -> Vec<usize> { vec![self.entries.len()] }
    fn rows(&self) -> usize { self.entries.len() }
    fn bytes(&self) -> TableBytes {
        const { assert!(!std::mem::needs_drop::<K>(), "regional keys must have no nested allocations"); }
        let mut total = TableBytes::vector(self.entries);
        for value in self.entries.iter().filter_map(Cache::stored) {
            let nested = (self.nested_bytes)(value);
            total.used = total.used.and_then(|n| n.checked_add(nested.used?));
            total.reserved = total.reserved.and_then(|n| n.checked_add(nested.reserved?));
        }
        total
    }
    fn columns(&self) -> Vec<Column> { cache_columns() }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.entries.len()).map(|region| (region, cache_cells(&self.entries[region]))).collect()
    }
    fn note(&self) -> Option<String> { Some("Original regional slots plus nested result buffers; dependency keys contain no heap allocations. Offscreen and invalidated results remain counted.".into()) }
}
fn cache_columns() -> Vec<Column> {
    vec![Column { name: "calculated_at_tt_jd", dtype: "Option<f64>" }, Column { name: "has_been_invalidated", dtype: "bool" },
        Column { name: "generation", dtype: "u64" }, Column { name: "dependencies", dtype: "key" }, Column { name: "result", dtype: "value" }]
}
fn cache_cells<K: Preview, V: Preview>(cache: &Cache<K, V>) -> Vec<String> {
    vec![format!("{:?}", cache.calculated_at), cache.has_been_invalidated.to_string(), cache.generation.to_string(),
        preview(&cache.key()), preview(&cache.stored())]
}
pub(super) fn no_nested_bytes<T>(_: &T) -> TableBytes { TableBytes::known(0, 0) }

pub(super) struct ObservationRegionsTable<'a>(pub &'a Vec<crate::model::ObservationRegion>);
impl Table for ObservationRegionsTable<'_> {
    fn shape(&self) -> Vec<usize> { vec![self.0.len()] }
    fn rows(&self) -> usize { self.0.len() }
    fn bytes(&self) -> TableBytes {
        let mut total = TableBytes::vector(self.0);
        for region in self.0 {
            let nested = observation_nested_bytes(region);
            total.used = total.used.and_then(|n| n.checked_add(nested.used?));
            total.reserved = total.reserved.and_then(|n| n.checked_add(nested.reserved?));
        }
        total
    }
    fn columns(&self) -> Vec<Column> {
        ["brightness_cache", "correction_cache", "aberration_cache"].map(|name| Column { name, dtype: "regional cache" }).into()
    }
    fn preview(&self) -> Vec<(usize, Vec<String>)> {
        preview_indices(self.0.len()).map(|index| {
            let region = &self.0[index];
            (index, vec![cache_cells(&region.eligible).join("; "), cache_cells(&region.corrections).join("; "), cache_cells(&region.apparent).join("; ")])
        }).collect()
    }
    fn note(&self) -> Option<String> { Some("Original regional correction owner; cache slots counted once, including all retained result allocations.".into()) }
}
pub(crate) fn observation_nested_bytes(region: &crate::model::ObservationRegion) -> TableBytes {
    let mut sizes = [TableBytes::known(0, 0); 3];
    if let Some(value) = region.eligible.stored() { sizes[0] = TableBytes::vector(value); }
    if let Some(value) = region.corrections.stored() { sizes[1] = TableBytes::vector(&value.0); }
    if let Some(value) = region.apparent.stored() { sizes[2] = TableBytes::vector(value); }
    TableBytes { used: sizes.iter().try_fold(0usize, |n, v| n.checked_add(v.used?)), reserved: sizes.iter().try_fold(0usize, |n, v| n.checked_add(v.reserved?)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_table_counts_offscreen_and_invalidated_payloads_once() {
        let mut entries: Vec<Cache<u64, Vec<usize>>> = (0..100).map(|_| Cache::default()).collect();
        entries[42].store(1, 0.0, 0.0, vec![3, 4, 5]); entries[42].invalidate();
        let table = RegionalTable { entries: &entries, nested_bytes: TableBytes::vector };
        assert_eq!(table.bytes().used, Some(entries.len() * size_of::<Cache<u64, Vec<usize>>>() + 3 * size_of::<usize>()));
        assert!(table.preview().len() <= 20);
        assert_eq!(table.rows(), 100);
    }
    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn regional_inventory_sums_all_payloads_without_exhausting_the_row_cap() {
        use crate::{astro::{Matrix3, Vector3}, cache::{Kind, ReportBuffers}, model::{SelectedStar, CorrectionStats, View, ProjectionViewport}, state::{ObservationCache, ProjectionCache}};
        fn used(owner: &impl ReportBuffers) -> usize {
            let snapshot = crate::state::collect_inventory("regional", owner);
            assert_eq!(snapshot.omitted_nodes, 0);
            assert!(snapshot.rows.len() < 200);
            snapshot.rows.iter().filter(|row| row.kind == Kind::Heap).map(|row| row.used.unwrap()).sum()
        }
        let mut observation = ObservationCache { regions: (0..crate::constants::SIMULATION_REGION_COUNT).map(|_| Default::default()).collect(), ..Default::default() };
        let entry = &mut observation.regions[123];
        entry.eligible.store((1, 1, 5.0), 0.0, 0.0, vec![true, false]);
        entry.corrections.store((1, 1), 0.0, 0.0, (vec![SelectedStar { source_index: 42, drawable: true }], CorrectionStats::default()));
        entry.apparent.store((1, 1, Vector3::default()), 0.0, 0.0, vec![Vector3::default()]);
        entry.apparent.invalidate(); // retained invalidated data must remain in the accounting
        assert_eq!(used(&observation), ObservationRegionsTable(&observation.regions).bytes().used.unwrap());

        let mut projection = ProjectionCache {
            regional_stars: (0..crate::constants::SIMULATION_REGION_COUNT).map(|_| Cache::default()).collect(),
            regional_orders: (0..crate::constants::SIMULATION_REGION_COUNT).map(|_| Cache::default()).collect(), ..Default::default()
        };
        projection.regional_stars[123].store(((1, 1, 1), Matrix3::IDENTITY, false, View::default(), ProjectionViewport { width: 10, height: 10 }), 0.0, 0.0, vec![crate::model::DrawnStar { source_index: 42, color: [1, 2, 3], cell: (3, 4), magnitude: 4.0 }]);
        projection.regional_orders[123].store((1, 1), 0.0, 0.0, vec![crate::model::RegionalDrawRecord { row: 0, source_index: 42, magnitude: 4.0 }]);
        let cells = RegionalTable { entries: &projection.regional_stars, nested_bytes: TableBytes::vector };
        let orders = RegionalTable { entries: &projection.regional_orders, nested_bytes: TableBytes::vector };
        assert_eq!(used(&projection), cells.bytes().used.unwrap() + orders.bytes().used.unwrap());
    }

}
