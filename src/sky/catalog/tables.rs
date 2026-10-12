//! Build the rebuilt pipeline's `SkyTables` from a prepared catalog (plan section 4, chunk 6). The prepared rows
//! are sorted by region and brightness; splitting each region's rows by speed group in that order keeps every
//! group table brightest first, so no new sort is needed. The constellation region becomes the constellation
//! table and the figures are renumbered to its rows. The result is validated before it is returned.
//!
//! Until the frame loop takes the tables over (chunk 13), the prepared catalog stays in memory as well; after
//! that, the split consumes it.
use std::io;

use crate::catalog::cache::invalid;
use crate::constants::{CELL_COUNT, CONSTELLATION_REGION, SIMULATION_REGION_COUNT, SPEED_GROUP_COUNT};
use crate::model::{Bodies, Constellation, Constellations, GroupTable, LabelColumn, PreparedCatalog, SkyCatalog, SkyTables, StarRow, StarRowVec, Stars};
use crate::progress::{DatasetProgress, Outcome, Stage, Unit};
use crate::regions::speed_group;
use crate::timing::StepTimes;

/// Split a prepared catalog into the per-group tables and the constellation table, then validate them.
pub fn build_sky_tables(prepared: &PreparedCatalog, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<SkyTables> {
    let count = prepared.catalog.stars.len();
    progress.begin_stage(Stage::Group, "", Unit::Stars, Some(count as u64));
    let result = build_tables(prepared, progress, times);
    progress.finish_stage(if result.is_ok() { Outcome::Complete } else { Outcome::Failed });
    result
}

fn build_tables(prepared: &PreparedCatalog, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<SkyTables> {
    // the inputs: brightness-sorted regions and the startup-only motion bounds
    let catalog = &prepared.catalog;
    let bounds = prepared.preparation.motion_bounds();
    let count = catalog.stars.len();
    if bounds.len() != count { return Err(invalid(format!("{} motion bounds for {count} stars; the group split needs the preparation data", bounds.len()))); }
    let offsets = &catalog.grid.offsets;
    if offsets.len() != SIMULATION_REGION_COUNT + 1 || offsets[SIMULATION_REGION_COUNT] != count {
        return Err(invalid("region offsets do not cover the prepared stars"));
    }

    // split, renumber, validate
    let groups = times.measure("Speed group split", || split_groups(catalog, bounds, progress));
    let constellations = times.measure("Constellation table", || build_constellations(catalog))?;
    let tables = SkyTables { bodies: Bodies::default(), stars: Stars::new(groups), constellations, names: catalog.names.clone(), clipping: catalog.stars.magnitude_clipping() };
    progress.set_detail("Checking the group tables");
    times.measure("Group table validation", || tables.validate())?;
    times.describe("Speed group split", || describe(&tables));
    Ok(tables)
}

/// One table per region and group, filled in the prepared order so that each table is brightest first.
fn split_groups(catalog: &SkyCatalog, bounds: &[f32], progress: &mut dyn DatasetProgress) -> Vec<GroupTable> {
    let columns = catalog.stars.columns();
    let names = catalog.stars.name_entries();
    let source_ids = catalog.stars.source_ids();
    let offsets = &catalog.grid.offsets;
    let mut tables = Vec::with_capacity(CELL_COUNT * SPEED_GROUP_COUNT);
    let mut groups: Vec<u8> = Vec::new();                                  // the group of every row of the current region
    for region in 0..CELL_COUNT {
        let range = offsets[region]..offsets[region + 1];
        groups.clear();
        groups.extend(range.clone().map(|row| speed_group(bounds[row]) as u8));
        for group in 0..SPEED_GROUP_COUNT {
            let members = || range.clone().zip(&groups).filter(move |&(_, &member)| usize::from(member) == group).map(|(row, _)| row);
            let size = members().count();
            let mut rows = StarRowVec::with_capacity(size);
            let mut labels = match source_ids { Some(_) => LabelColumn::SourceIds(Vec::with_capacity(size)), None => LabelColumn::Names(Vec::with_capacity(size)) };
            for row in members() {
                rows.push(star_row(&columns, row).into());
                match (&mut labels, source_ids) {
                    (LabelColumn::SourceIds(ids), Some(source)) => ids.push(source[row]),
                    (LabelColumn::Names(entries), None) => entries.push(names[row]),
                    _ => unreachable!("label kind chosen from the same catalog"),
                }
            }
            tables.push(GroupTable::new(rows, labels));
        }
        if region % 1024 == 1023 { progress.set_position(range.end as u64); }
    }
    progress.set_position(offsets[CELL_COUNT] as u64);
    tables
}

/// The constellation region as its own table, with the figures renumbered to its rows.
fn build_constellations(catalog: &SkyCatalog) -> io::Result<Constellations> {
    let columns = catalog.stars.columns();
    let names = catalog.stars.name_entries();
    let (start, end) = (catalog.grid.offsets[CONSTELLATION_REGION], catalog.grid.offsets[CONSTELLATION_REGION + 1]);
    let mut rows = StarRowVec::with_capacity(end - start);
    let mut entries = Vec::with_capacity(end - start);
    for row in start..end {
        rows.push(star_row(&columns, row).into());
        entries.push(names.get(row).copied().unwrap_or(0));
    }
    let renumber = |row: usize| (start..end).contains(&row).then(|| row - start)
        .ok_or_else(|| invalid(format!("constellation endpoint at row {row} lies outside the constellation region {start}..{end}")));
    let figures = catalog.constellations().iter().map(|figure| {
        let segments = figure.segments.iter().map(|&[a, b]| Ok([renumber(a)?, renumber(b)?])).collect::<io::Result<Vec<_>>>()?;
        Ok(Constellation { abbreviation: figure.abbreviation, segments })
    }).collect::<io::Result<Vec<_>>>()?;
    Ok(Constellations::new(rows, entries, figures))
}

fn star_row(columns: &crate::model::StarRowSlice<'_>, row: usize) -> StarRow {
    StarRow {
        u0: columns.u0[row], w: columns.w[row], magnitude: columns.magnitude[row], brightness_key: columns.brightness_key[row],
        distance: columns.distance[row], id: columns.id[row], name: 0, display_color: columns.display_color[row],
    }
}

fn describe(tables: &SkyTables) -> String {
    let mut per_group = [0usize; SPEED_GROUP_COUNT];
    let mut filled = 0;
    for (_, group, table) in tables.stars.iter() {
        per_group[group] += table.len();
        filled += usize::from(!table.is_empty());
    }
    format!("slow={}; medium={}; fast={}; constellation={}; filled group tables={filled} of {}", per_group[0], per_group[1], per_group[2],
        tables.constellations.len(), CELL_COUNT * SPEED_GROUP_COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{NameId, load_embedded_catalog};
    use crate::progress::NoProgress;

    fn embedded() -> PreparedCatalog {
        crate::sky::prepare_owned_catalog(load_embedded_catalog().unwrap()).unwrap()
    }

    fn build(prepared: &PreparedCatalog) -> io::Result<SkyTables> {
        build_sky_tables(prepared, &mut NoProgress(io::sink()), &mut StepTimes::default())
    }

    /// Where the named star is: its speed group, or `None` for the constellation table.
    fn group_of(tables: &SkyTables, name: &str) -> Option<usize> {
        let named = |entry: u32| tables.names.get(NameId::from_entry(entry)) == Some(name);
        for (_, group, table) in tables.stars.iter() {
            let LabelColumn::Names(entries) = table.labels() else { panic!("stored catalog") };
            if entries.iter().any(|&entry| named(entry)) { return Some(group); }
        }
        assert!(tables.constellations.names().iter().any(|&entry| named(entry)), "{name} is in no table");
        None
    }

    #[test]
    fn embedded_catalog_splits_into_validated_tables() {
        let prepared = embedded();
        let tables = build(&prepared).unwrap();
        let catalog = &prepared.catalog;
        assert_eq!(tables.stars.len() + tables.constellations.len(), catalog.stars.len());
        assert_eq!(tables.constellations.len(), catalog.endpoint_indices().len());
        assert_eq!(tables.constellations.figures().len(), catalog.constellations().len());
        assert_eq!(tables.names, catalog.names);
        for region in 0..CELL_COUNT {                                                   // every region keeps its stars, split over its three tables
            let total: usize = tables.stars.region(region).iter().map(GroupTable::len).sum();
            assert_eq!(total, catalog.grid.offsets[region + 1] - catalog.grid.offsets[region], "region {region}");
        }
        assert_eq!(tables.bodies.planets.len(), 8);
    }

    #[test]
    fn well_known_stars_land_in_the_expected_speed_groups() {
        let tables = build(&embedded()).unwrap();
        assert_eq!(group_of(&tables, "Hatysa"), Some(0));                               // 1 mas/yr
        assert_eq!(group_of(&tables, "Almaaz"), Some(0));                               // 4 mas/yr
        assert_eq!(group_of(&tables, "Alcyone"), Some(1));                              // 49 mas/yr
        assert_eq!(group_of(&tables, "Alcor"), Some(1));                                // 70 mas/yr
        assert_eq!(group_of(&tables, "Yed Prior"), Some(1));                            // 0.15″/yr
        assert_eq!(group_of(&tables, "Titawin"), Some(2));                              // 0.41″/yr
        assert_eq!(group_of(&tables, "Toliman"), Some(2));                              // 1.9″/yr, α Centauri
        assert_eq!(group_of(&tables, "Keid"), Some(2));                                 // 4.1″/yr
        for endpoint in ["Sirius", "Polaris", "Deneb", "Arcturus"] { assert_eq!(group_of(&tables, endpoint), None, "{endpoint}"); } // constellation stars are in no group
        let counts = (0..SPEED_GROUP_COUNT).map(|g| tables.stars.iter().filter(|&(_, group, _)| group == g).map(|(_, _, t)| t.len()).sum::<usize>()).collect::<Vec<_>>();
        assert_eq!(counts, [4069, 4096, 239]);                                         // BSC5 without the 706 constellation endpoints
    }

    #[test]
    fn tables_from_the_prepared_cache_equal_tables_from_preparation() {
        let prepared = embedded();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsc5.catalog");
        let fingerprint = [7_u8; 32];
        crate::sky::write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
        let cached = crate::sky::load_cached_catalog(&path, &fingerprint).unwrap();
        assert_eq!(build(&cached).unwrap(), build(&prepared).unwrap());
    }

    #[test]
    fn missing_preparation_data_is_rejected() {
        let mut prepared = embedded();
        prepared.preparation.motion_bounds.clear();
        assert!(build(&prepared).unwrap_err().to_string().contains("0 motion bounds"));
    }

    #[test]
    fn empty_catalog_gives_empty_tables() {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(Vec::new(), crate::catalog::StarNames::default(), Vec::new())).unwrap();
        let tables = build(&prepared).unwrap();
        assert!(tables.stars.is_empty() && tables.constellations.is_empty());
        assert_eq!(describe(&tables), format!("slow=0; medium=0; fast=0; constellation=0; filled group tables=0 of {}", CELL_COUNT * SPEED_GROUP_COUNT));
    }
}
