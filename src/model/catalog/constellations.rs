//! Validate the endpoint stars and line connections owned by Data. Constellation endpoints never also belong
//! to ordinary regions; their whole table is always considered, and Gaia-only catalogs leave it empty.
use std::io;

use crate::catalog::cache::invalid;
use crate::model::Data;

impl Data {
    /// IDs used to ensure constellation endpoints never also appear in ordinary regions.
    pub fn constellation_ids(&self) -> Vec<u32> {
        let mut ids = self.constellation_stars.columns().id.to_vec();
        ids.sort_unstable();
        ids
    }

    /// Check that the figures and the table describe each other: every segment end is a row, every row is the
    /// end of some segment, no star appears twice, and the name column matches the star count.
    pub fn validate_constellations(&self) -> io::Result<()> {
        let count = self.constellation_stars.len();
        if self.constellation_stars.name_entries().len() != count { return Err(invalid(format!("{} constellation star names for {count} stars", self.constellation_stars.name_entries().len()))); }
        let mut referenced = vec![false; count];
        for figure in &self.constellation_figures {
            for &index in figure.segments.iter().flatten() {
                let slot = referenced.get_mut(index).ok_or_else(|| invalid(format!("constellation {} refers to star row {index} of {count}", figure.abbreviation)))?;
                *slot = true;
            }
        }
        if let Some(row) = referenced.iter().position(|&used| !used) {
            return Err(invalid(format!("constellation star {} at row {row} is not an endpoint of any figure", self.constellation_stars.id(row).0)));
        }
        let ids = self.constellation_ids();
        if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(invalid(format!("constellation star {} is stored twice", pair[0])));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::StarId;
    use crate::model::{Constellation, StarTable, StarLabels, StarRowVec};
    use super::*;
    use super::super::stars::fixtures::{region_center, row};

    fn triangle_with(ids: [u32; 3], names: Vec<u32>) -> Data {
        let mut columns = StarRowVec::new();
        for (region, (id, magnitude)) in [21_usize, 22, 23].into_iter().zip(ids.into_iter().zip([0.5, 2.5, 1.0])) { columns.push(row(region_center(region), 0.0, magnitude, id).into()); }
        let figures = vec![Constellation { abbreviation: "Tri", segments: vec![[0, 1], [1, 2], [2, 0]] }];
        Data { constellation_stars: StarTable::new(columns, StarLabels::Names(names)), constellation_figures: figures, ..Data::default() }
    }
    fn triangle() -> Data { triangle_with([21, 22, 23], vec![3, 0, 0]) }

    #[test]
    fn empty_table_validates() {
        let empty = Data::default();
        empty.validate_constellations().unwrap();
        assert!(empty.constellation_stars.is_empty());
        assert!(empty.constellation_ids().is_empty());
    }

    #[test]
    fn figures_and_rows_describe_each_other() {
        let table = triangle();
        table.validate_constellations().unwrap();
        assert_eq!(table.constellation_stars.len(), 3);
        assert_eq!(table.constellation_ids(), [21, 22, 23]);
        assert_eq!(table.constellation_stars.id(2), StarId(23));
        assert_eq!(table.constellation_stars.name_entries(), [3, 0, 0]);
        assert_eq!(table.constellation_figures[0].segments.len(), 3);
        assert!((table.constellation_stars.motion(1).u0 - region_center(22)).length() < 1e-6);   // stored as f32
    }

    #[test]
    fn validation_rejects_dangling_segments_unused_rows_and_duplicates() {
        let mut dangling = triangle();
        dangling.constellation_figures[0].segments.push([2, 3]);
        assert!(dangling.validate_constellations().unwrap_err().to_string().contains("star row 3 of 3"));
        let mut unused = triangle();
        unused.constellation_figures[0].segments.truncate(1);
        assert!(unused.validate_constellations().unwrap_err().to_string().contains("star 23 at row 2 is not an endpoint"));
        let twice = triangle_with([21, 22, 21], vec![3, 0, 0]);
        assert!(twice.validate_constellations().unwrap_err().to_string().contains("star 21 is stored twice"));
        let names = triangle_with([21, 22, 23], vec![3, 0]);
        assert!(names.validate_constellations().unwrap_err().to_string().contains("2 constellation star names for 3 stars"));
    }
}
