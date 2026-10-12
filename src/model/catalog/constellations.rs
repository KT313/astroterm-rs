//! The constellation table of the pipeline rebuild (plan section 4): every constellation endpoint star, stored
//! here and nowhere else, and the figures as segments between rows of this table. It may be empty (a Gaia-only
//! run has no figures). The frame always selects the whole table, so it has no regions or groups.
use std::io;

use crate::astro::{Vector3, models::stars::StellarMotion};
use crate::catalog::{StarId, cache::invalid};
use super::{Constellation, StarRowSlice, StarRowVec};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Constellations {
    columns: StarRowVec,
    names: Vec<u32>,                       // name entries, one-based into the shared `StarNames`; zero for none
    figures: Vec<Constellation>,           // segments index rows of `columns`
}

impl Constellations {
    /// Assemble the table from its parts without checking them; the loader calls `validate` afterwards.
    pub fn new(columns: StarRowVec, names: Vec<u32>, figures: Vec<Constellation>) -> Self {
        Self { columns, names, figures }
    }

    pub fn len(&self) -> usize { self.columns.len() }
    pub fn is_empty(&self) -> bool { self.columns.is_empty() }
    pub fn columns(&self) -> StarRowSlice<'_> { self.columns.as_slice() }
    pub fn names(&self) -> &[u32] { &self.names }
    pub fn figures(&self) -> &[Constellation] { &self.figures }
    pub fn id(&self, i: usize) -> StarId { StarId(self.columns.id[i]) }

    pub fn motion(&self, i: usize) -> StellarMotion {
        let (u0, w, distance) = (self.columns.u0[i], self.columns.w[i], self.columns.distance[i]);
        StellarMotion { u0: expand(u0), w: expand(w), distance_pc: (distance > 0.0).then_some(f64::from(distance)) }
    }

    /// The IDs of the table's stars in ascending order, for the region table's exclusion check.
    pub fn sorted_ids(&self) -> Vec<u32> {
        let mut ids = self.columns.id.to_vec();
        ids.sort_unstable();
        ids
    }

    /// Check that the figures and the table describe each other: every segment end is a row, every row is the
    /// end of some segment, no star appears twice, and the name column matches the star count.
    pub fn validate(&self) -> io::Result<()> {
        let count = self.len();
        if self.names.len() != count { return Err(invalid(format!("{} constellation star names for {count} stars", self.names.len()))); }
        let mut referenced = vec![false; count];
        for figure in &self.figures {
            for &index in figure.segments.iter().flatten() {
                let slot = referenced.get_mut(index).ok_or_else(|| invalid(format!("constellation {} refers to star row {index} of {count}", figure.abbreviation)))?;
                *slot = true;
            }
        }
        if let Some(row) = referenced.iter().position(|&used| !used) {
            return Err(invalid(format!("constellation star {} at row {row} is not an endpoint of any figure", self.columns.id[row])));
        }
        let ids = self.sorted_ids();
        if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(invalid(format!("constellation star {} is stored twice", pair[0])));
        }
        Ok(())
    }
}

fn expand(v: [f32; 3]) -> Vector3 {
    Vector3 { x: f64::from(v[0]), y: f64::from(v[1]), z: f64::from(v[2]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::stars::fixtures::{region_center, row};

    fn triangle() -> Constellations {
        let mut columns = StarRowVec::new();
        for (id, magnitude) in [(21_u32, 0.5), (22, 2.5), (23, 1.0)] { columns.push(row(region_center(id as usize), 0.0, magnitude, id).into()); }
        let figures = vec![Constellation { abbreviation: "Tri", segments: vec![[0, 1], [1, 2], [2, 0]] }];
        Constellations::new(columns, vec![3, 0, 0], figures)
    }

    #[test]
    fn empty_table_validates() {
        let empty = Constellations::default();
        empty.validate().unwrap();
        assert!(empty.is_empty());
        assert!(empty.sorted_ids().is_empty());
    }

    #[test]
    fn figures_and_rows_describe_each_other() {
        let table = triangle();
        table.validate().unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table.sorted_ids(), [21, 22, 23]);
        assert_eq!(table.id(2), StarId(23));
        assert_eq!(table.names(), [3, 0, 0]);
        assert_eq!(table.figures()[0].segments.len(), 3);
        assert!((table.motion(1).u0 - region_center(22)).length() < 1e-6);   // stored as f32
    }

    #[test]
    fn validation_rejects_dangling_segments_unused_rows_and_duplicates() {
        let mut dangling = triangle();
        dangling.figures[0].segments.push([2, 3]);
        assert!(dangling.validate().unwrap_err().to_string().contains("star row 3 of 3"));
        let mut unused = triangle();
        unused.figures[0].segments.truncate(1);
        assert!(unused.validate().unwrap_err().to_string().contains("star 23 at row 2 is not an endpoint"));
        let mut twice = triangle();
        twice.columns.id[2] = 21;
        assert!(twice.validate().unwrap_err().to_string().contains("star 21 is stored twice"));
        let mut names = triangle();
        names.names.pop();
        assert!(names.validate().unwrap_err().to_string().contains("2 constellation star names for 3 stars"));
    }
}
