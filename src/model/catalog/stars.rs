//! The star structure of the pipeline rebuild (plan section 4): every speed group of every region owns its own
//! table of stars, sorted brightest first. Built once at startup and never modified; the caches of the rebuilt
//! pipeline are a second list with the same numbering, so a worker can own whole groups later on.
//!
//! A star is in the table of the region its stored direction hashes to (`hash_direction`) and of the speed group
//! its stored motion bound gives (`crate::model::speed_group`). Constellation endpoint stars are in no group; they live
//! in [`Constellations`](super::Constellations).
use std::io;

use crate::astro::{Vector3, models::stars::StellarMotion};
use crate::catalog::{Catalog, StarId, cache::invalid, passes_brightness_bound};
use crate::constants::{CELL_COUNT, GRID_DEPTH, QUANTIZATION_MARGIN, SPEED_GROUP_COUNT};
use crate::model::speed_group;
use super::{StarRowSlice, StarRowVec, hash_direction};

/// How the stars of a table are labelled: name entries (one-based into the shared `StarNames`, zero for none) for
/// stored catalogs, or Gaia source IDs from which the label "Gaia DR3 …" is derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelColumn { Names(Vec<u32>), SourceIds(Vec<u64>) }
impl LabelColumn {
    pub fn len(&self) -> usize { match self { Self::Names(entries) => entries.len(), Self::SourceIds(ids) => ids.len() } }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

/// The stars of one speed group of one region, brightest first. Row numbers are local to the table.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupTable {
    columns: StarRowVec,
    labels: LabelColumn,
}

impl Default for GroupTable {
    fn default() -> Self { Self { columns: StarRowVec::new(), labels: LabelColumn::Names(Vec::new()) } }
}

impl GroupTable {
    /// Assemble a table from its parts without checking them; `Stars::validate` checks all tables afterwards.
    pub fn new(columns: StarRowVec, labels: LabelColumn) -> Self { Self { columns, labels } }

    pub fn len(&self) -> usize { self.columns.len() }
    pub fn is_empty(&self) -> bool { self.columns.is_empty() }
    pub fn columns(&self) -> StarRowSlice<'_> { self.columns.as_slice() }
    pub fn labels(&self) -> &LabelColumn { &self.labels }
    pub fn id(&self, row: usize) -> StarId { StarId(self.columns.id[row]) }
    pub fn stored_direction(&self, row: usize) -> Vector3 { expand(self.columns.u0[row]) }

    pub fn motion(&self, row: usize) -> StellarMotion {
        let distance = self.columns.distance[row];
        StellarMotion { u0: expand(self.columns.u0[row]), w: expand(self.columns.w[row]), distance_pc: (distance > 0.0).then_some(f64::from(distance)) }
    }

    /// The motion bound the loader stores for a row: its largest drift over the computational interval plus the
    /// quantization margin, rounded up. Group membership is decided from this value and nothing else.
    pub fn motion_bound(&self, row: usize) -> f32 {
        ((self.motion(row).motion_bound() + QUANTIZATION_MARGIN) as f32).next_up()
    }

    /// How many of the first rows have a brightness key passing `threshold` (an inclusive magnitude limit): the
    /// group's brightness prefix, because the rows are sorted by brightness key.
    pub fn bright_count(&self, threshold: f64) -> usize {
        self.columns.brightness_key.partition_point(|&key| passes_brightness_bound(key, threshold))
    }
}

/// All group tables, numbered region-major: the table of group `g` in region `r` is entry `r × SPEED_GROUP_COUNT + g`.
#[derive(Clone, Debug, PartialEq)]
pub struct Stars {
    groups: Vec<GroupTable>,                // CELL_COUNT × SPEED_GROUP_COUNT tables; empty groups are empty tables
}

impl Default for Stars {
    /// No stars: every group is an empty table.
    fn default() -> Self { Self::new((0..CELL_COUNT * SPEED_GROUP_COUNT).map(|_| GroupTable::default()).collect()) }
}

impl Stars {
    /// Assemble the structure from its tables without checking them; the loader calls `validate` afterwards.
    pub fn new(groups: Vec<GroupTable>) -> Self { Self { groups } }

    /// Stars in all tables together.
    pub fn len(&self) -> usize { self.groups.iter().map(GroupTable::len).sum() }
    pub fn is_empty(&self) -> bool { self.groups.iter().all(GroupTable::is_empty) }

    /// The table of one speed group of one region.
    pub fn group(&self, region: usize, group: usize) -> &GroupTable { &self.groups[region * SPEED_GROUP_COUNT + group] }

    /// The tables of one region, slow to fast.
    pub fn region(&self, region: usize) -> &[GroupTable] { &self.groups[region * SPEED_GROUP_COUNT..(region + 1) * SPEED_GROUP_COUNT] }

    /// Every table with its region and group number, region-major.
    pub fn iter(&self) -> impl Iterator<Item = (usize, usize, &GroupTable)> {
        self.groups.iter().enumerate().map(|(index, table)| (index / SPEED_GROUP_COUNT, index % SPEED_GROUP_COUNT, table))
    }

    /// Check everything the structure promises: one table per region and group, every row in the region of its
    /// direction and the group of its motion bound, every table brightest first with as many labels as rows, and
    /// none of the `constellation_ids` (sorted) among the rows.
    pub fn validate(&self, constellation_ids: &[u32]) -> io::Result<()> {
        if self.groups.len() != CELL_COUNT * SPEED_GROUP_COUNT {
            return Err(invalid(format!("{} group tables, expected {}", self.groups.len(), CELL_COUNT * SPEED_GROUP_COUNT)));
        }
        Catalog::check_star_count(self.len() as u64).map_err(io::Error::other)?;
        for (region, group, table) in self.iter() {
            if table.labels.len() != table.len() {
                return Err(invalid(format!("region {region}, group {group}: {} labels for {} stars", table.labels.len(), table.len())));
            }
            let columns = table.columns();
            for row in 0..table.len() {
                let id = columns.id[row];
                let actual = hash_direction(GRID_DEPTH, table.stored_direction(row));
                if actual != region { return Err(invalid(format!("star {id} is in the table of region {region} but points into region {actual}"))); }
                let actual = speed_group(table.motion_bound(row));
                if actual != group { return Err(invalid(format!("star {id} is in the table of speed group {group} but moves like group {actual}"))); }
                if row > 0 && columns.brightness_key[row] < columns.brightness_key[row - 1] {
                    return Err(invalid(format!("star {id} is brighter than the star before it in its table")));
                }
                if constellation_ids.binary_search(&id).is_ok() { return Err(invalid(format!("constellation star {id} is also in a table of region {region}"))); }
            }
        }
        Ok(())
    }
}

fn expand(v: [f32; 3]) -> Vector3 {
    Vector3 { x: f64::from(v[0]), y: f64::from(v[1]), z: f64::from(v[2]) }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Hand-built tiny catalogs for the layout tests of this folder.
    use super::*;
    use crate::catalog::encode_magnitude;
    use crate::model::StarRow;

    const ARCSECOND: f64 = std::f64::consts::PI / (180.0 * 3600.0);

    /// A row pointing along `direction`, moving sideways at `arcseconds_per_year`, with no distance.
    pub(crate) fn row(direction: Vector3, arcseconds_per_year: f64, magnitude: f64, id: u32) -> StarRow {
        let u0 = direction.normalized();
        let axis = if u0.z.abs() < 0.9 { Vector3 { x: 0.0, y: 0.0, z: 1.0 } } else { Vector3 { x: 1.0, y: 0.0, z: 0.0 } };
        let w = u0.cross(axis).normalized() * (arcseconds_per_year * ARCSECOND);
        let code = encode_magnitude(magnitude).unwrap();
        StarRow { u0: pack(u0), w: pack(w), magnitude: code, brightness_key: code, distance: 0.0, id, name: 0, display_color: 0 }
    }

    /// The centre direction of a region.
    pub(crate) fn region_center(region: usize) -> Vector3 { super::super::grid::build_caps(GRID_DEPTH)[region].center }

    /// Put rows into the tables claimed for them, in the given order. Nothing is checked, so tests can build wrong
    /// layouts on purpose.
    pub(crate) fn layout(placed: &[(usize, usize, StarRow)]) -> Stars {
        let mut stars = Stars::default();
        for &(region, group, row) in placed {
            let table = &mut stars.groups[region * SPEED_GROUP_COUNT + group];
            table.columns.push(row.into());
            let LabelColumn::Names(names) = &mut table.labels else { unreachable!() };
            names.push(row.name);
        }
        stars
    }

    fn pack(v: Vector3) -> [f32; 3] { [v.x as f32, v.y as f32, v.z as f32] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::fixtures::{layout, region_center, row};

    /// Two regions: `a` with a slow pair and one fast star, `b` with one slow and two medium stars.
    fn two_regions() -> (Stars, usize, usize) {
        let (a, b) = (hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.1, z: 0.1 }), hash_direction(GRID_DEPTH, Vector3 { x: -0.2, y: 1.0, z: 0.3 }));
        let stars = layout(&[
            (a, 0, row(region_center(a), 0.0, 1.0, 1)),
            (a, 0, row(region_center(a), 0.01, 3.0, 2)),
            (a, 2, row(region_center(a), 1.0, 2.0, 3)),
            (b, 0, row(region_center(b), 0.0, 4.0, 4)),
            (b, 1, row(region_center(b), 0.1, 0.5, 5)),
            (b, 1, row(region_center(b), 0.1, 5.0, 6)),
        ]);
        (stars, a, b)
    }

    #[test]
    fn empty_structure_validates_with_empty_tables() {
        let stars = Stars::default();
        stars.validate(&[]).unwrap();
        assert!(stars.is_empty());
        assert_eq!(stars.len(), 0);
        assert_eq!(stars.iter().count(), CELL_COUNT * SPEED_GROUP_COUNT);
        assert!(stars.group(CELL_COUNT - 1, SPEED_GROUP_COUNT - 1).is_empty());
        assert_eq!(stars.group(5, 1).bright_count(6.0), 0);
    }

    #[test]
    fn tables_hold_their_region_and_group_brightest_first() {
        let (stars, a, b) = two_regions();
        stars.validate(&[]).unwrap();
        assert_eq!(stars.len(), 6);
        assert_eq!(stars.region(a).iter().map(GroupTable::len).collect::<Vec<_>>(), [2, 0, 1]);   // an empty group between two filled ones
        assert_eq!(stars.region(b).iter().map(GroupTable::len).collect::<Vec<_>>(), [1, 2, 0]);
        let ids = |table: &GroupTable| (0..table.len()).map(|row| table.id(row).0).collect::<Vec<_>>();
        assert_eq!(ids(stars.group(a, 0)), [1, 2]);
        assert_eq!(ids(stars.group(b, 1)), [5, 6]);
        for (region, _, table) in stars.iter().filter(|&(region, _, _)| region != a && region != b) {
            assert!(table.is_empty(), "region {region}");
        }
        for (region, _, table) in stars.iter() {
            for row in 0..table.len() { assert_eq!(hash_direction(GRID_DEPTH, table.stored_direction(row)), region); }
        }
    }

    #[test]
    fn bright_count_cuts_each_table_at_the_threshold() {
        let (stars, a, b) = two_regions();
        assert_eq!(stars.group(a, 0).bright_count(0.5), 0);
        assert_eq!(stars.group(a, 0).bright_count(1.0), 1);                 // the limit is inclusive
        assert_eq!(stars.group(a, 0).bright_count(2.0), 1);
        assert_eq!(stars.group(a, 0).bright_count(3.0), 2);
        assert_eq!(stars.group(a, 1).bright_count(10.0), 0);
        assert_eq!(stars.group(a, 2).bright_count(1.9), 0);
        assert_eq!(stars.group(b, 1).bright_count(0.5), 1);
        assert_eq!(stars.group(b, 1).bright_count(5.0), 2);
    }

    #[test]
    fn validation_rejects_rows_in_the_wrong_table_or_order() {
        let (a, b) = (hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.1, z: 0.1 }), hash_direction(GRID_DEPTH, Vector3 { x: -0.2, y: 1.0, z: 0.3 }));
        let misplaced = layout(&[(b, 0, row(region_center(a), 0.0, 1.0, 7))]);
        assert!(misplaced.validate(&[]).unwrap_err().to_string().contains("points into region"));
        let fast_in_slow = layout(&[(a, 0, row(region_center(a), 1.0, 1.0, 8))]);
        assert!(fast_in_slow.validate(&[]).unwrap_err().to_string().contains("moves like group 2"));
        let medium_in_fast = layout(&[(a, 2, row(region_center(a), 0.1, 1.0, 9))]);
        assert!(medium_in_fast.validate(&[]).unwrap_err().to_string().contains("moves like group 1"));
        let faint_first = layout(&[(a, 0, row(region_center(a), 0.0, 3.0, 10)), (a, 0, row(region_center(a), 0.0, 1.0, 11))]);
        assert!(faint_first.validate(&[]).unwrap_err().to_string().contains("brighter than the star before it"));
    }

    #[test]
    fn validation_rejects_wrong_table_counts_and_labels() {
        let (stars, a, _) = two_regions();
        let mut missing = stars.clone();
        missing.groups.pop();
        assert!(missing.validate(&[]).unwrap_err().to_string().contains("group tables, expected"));
        let mut labels = stars.clone();
        labels.groups[a * SPEED_GROUP_COUNT].labels = LabelColumn::SourceIds(vec![1]);
        assert!(labels.validate(&[]).unwrap_err().to_string().contains("1 labels for 2 stars"));
    }

    #[test]
    fn constellation_stars_may_not_appear_in_any_table() {
        let (stars, _, _) = two_regions();
        stars.validate(&[7, 8]).unwrap();
        assert!(stars.validate(&[2, 9]).unwrap_err().to_string().contains("constellation star 2 is also in a table"));
    }
}
