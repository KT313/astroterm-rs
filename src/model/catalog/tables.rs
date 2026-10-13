//! Construction and validation for the unified catalog and runtime data owner.
use std::io;
use crate::constants::CELL_COUNT;
use crate::model::{Data, Region, SunTable, PlanetsTable, MoonsTable};

impl Default for Data {
    fn default() -> Self {
        Self {
            solar_system_sun: SunTable::default(),
            solar_system_planets: PlanetsTable::default(),
            solar_system_moons: MoonsTable::default(),
            constellation_stars: Default::default(),
            constellation_simulation: Default::default(),
            constellation_figures: Vec::new(),
            star_regions: (0..CELL_COUNT).map(|_| Region::default()).collect(),
            names: Default::default(),
            clipping: Default::default(),
            star_opacities: Default::default(),
            tile_bytes: 0,
        }
    }
}

impl Data {
    /// Check table placement, ordering, labels and constellation membership before running frames.
    pub fn validate(&self) -> io::Result<()> {
        self.validate_constellations()?;
        self.validate_regions(&self.constellation_ids())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::stars::fixtures::{layout, region_center, row};
    use crate::astro::Vector3;
    use crate::constants::GRID_DEPTH;
    use crate::model::{Constellation, StarRowVec, StarTable, StarLabels, hash_direction};

    #[test]
    fn empty_tables_validate() {
        let tables = Data::default();
        tables.validate().unwrap();
        assert!(tables.region_star_count() == 0 && tables.constellation_stars.is_empty());
        assert_eq!(tables.solar_system_planets, PlanetsTable::default());
        assert_eq!(tables.star_regions.len(), CELL_COUNT);
        assert_eq!(tables.tile_bytes, 0);
        assert_eq!(tables.constellation_simulation.last_update, None);
        for region in &tables.star_regions {
            assert_eq!(region.merged_tile.group_keys, None);
            for group in &region.speed_groups {
                assert_eq!(group.simulation.last_update, None);
                assert!(group.simulation.directions.is_empty());
                assert_eq!(group.tile.key, None);
                assert!(group.tile.tile.texels.is_empty());
            }
        }
    }

    #[test]
    fn a_star_may_sit_in_a_region_or_in_a_figure_but_not_both() {
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: -0.2, z: -1.0 });
        let mut columns = StarRowVec::new();
        for id in [31, 32] { columns.push(row(region_center(region), 0.0, 2.0, id).into()); }
        let figures = vec![Constellation { abbreviation: "Pair", segments: vec![[0, 1]] }];
        let mut tables = Data {
            star_regions: layout(&[(region, 0, row(region_center(region), 0.0, 1.0, 30))]).star_regions,
            constellation_stars: StarTable::new(columns, StarLabels::Names(vec![0, 0])),
            constellation_figures: figures,
            ..Data::default()
        };
        tables.validate().unwrap();
        tables.star_regions = layout(&[(region, 0, row(region_center(region), 0.0, 1.0, 30)), (region, 0, row(region_center(region), 0.0, 2.0, 32))]).star_regions;
        assert!(tables.validate().unwrap_err().to_string().contains("constellation star 32 is also in a table"));
    }
}
