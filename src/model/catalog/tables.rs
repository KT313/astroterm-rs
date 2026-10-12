//! Everything the rebuilt pipeline builds once at startup (plan section 4): the solar-system tables, the region
//! star structure, the constellation table and the shared name text. Immutable afterwards; the frame steps borrow it.
use std::io;

use crate::catalog::StarNames;
use super::{Bodies, Constellations, Stars};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkyTables {
    pub bodies: Bodies,
    pub stars: Stars,
    pub constellations: Constellations,
    pub names: StarNames,                  // label text for the name entries of both star tables
    pub clipping: crate::catalog::MagnitudeClipping, // catalog-wide counts of stored brightness bounds that hit the code range; drives the notice
}

impl SkyTables {
    /// Check both star tables and that no constellation star also sits in a region.
    pub fn validate(&self) -> io::Result<()> {
        self.constellations.validate()?;
        self.stars.validate(&self.constellations.sorted_ids())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::stars::fixtures::{layout, region_center, row};
    use crate::astro::Vector3;
    use crate::constants::GRID_DEPTH;
    use crate::model::{Constellation, StarRowVec, hash_direction};

    #[test]
    fn empty_tables_validate() {
        let tables = SkyTables::default();
        tables.validate().unwrap();
        assert!(tables.stars.is_empty() && tables.constellations.is_empty());
        assert_eq!(tables.bodies, Bodies::default());
    }

    #[test]
    fn a_star_may_sit_in_a_region_or_in_a_figure_but_not_both() {
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: -0.2, z: -1.0 });
        let mut columns = StarRowVec::new();
        for id in [31, 32] { columns.push(row(region_center(region), 0.0, 2.0, id).into()); }
        let figures = vec![Constellation { abbreviation: "Pair", segments: vec![[0, 1]] }];
        let mut tables = SkyTables {
            stars: layout(&[(region, 0, row(region_center(region), 0.0, 1.0, 30))]),
            constellations: Constellations::new(columns, vec![0, 0], figures),
            ..SkyTables::default()
        };
        tables.validate().unwrap();
        tables.stars = layout(&[(region, 0, row(region_center(region), 0.0, 1.0, 30)), (region, 0, row(region_center(region), 0.0, 2.0, 32))]);
        assert!(tables.validate().unwrap_err().to_string().contains("constellation star 32 is also in a table"));
    }
}
