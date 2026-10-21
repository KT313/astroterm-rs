//! The tiles of the cells a split region is drawn as (`Selected::leaves`, step 3's decision). Every frame, each
//! drawn star of the region goes to the leaf holding its current direction, and each leaf's tile is drawn as the
//! region's own would be: group by group, then merged. Not a cache: only the allocations outlive the frame.
use crate::constants::SPEED_GROUP_COUNT;
use crate::model::{ChildTile, GroupTileCache, GroupTileKey, SkyCell, StarGroup, StarOpacities};
use crate::regions::{TileGeometry, is_star_drawn};

/// Each leaf's rows of each group: `[leaf][group]`, row numbers of the group's simulation.
type LeafRows = Vec<[Vec<usize>; SPEED_GROUP_COUNT]>;

/// Draw the tiles of `leaves`, the in-view cells that split `region`, from the first `star_counts[group]` simulated
/// rows of each group at magnitude code `threshold`. A star goes to the leaf holding its current direction; one
/// that drifted out of the region goes to the nearest cell at the region's edge, and is lost when it lies beyond
/// that tile's glyph margin. Stars in cells out of view have no leaf and are not drawn.
pub(super) fn draw_child_tiles(child_tiles: &mut Vec<ChildTile>, groups: &[StarGroup; SPEED_GROUP_COUNT], region: usize, leaves: &[SkyCell], star_counts: [usize; SPEED_GROUP_COUNT], threshold: u16, opacities: &StarOpacities) {
    // Hand every drawn star to its leaf, then draw one tile per leaf, reusing the previous frame's allocations.
    let rows = sort_rows_into_leaves(groups, SkyCell::region(region), leaves, star_counts, threshold);
    child_tiles.resize_with(leaves.len(), ChildTile::default);
    let mut images: [GroupTileCache; SPEED_GROUP_COUNT] = Default::default();
    for ((child, &leaf), leaf_rows) in child_tiles.iter_mut().zip(leaves).zip(&rows) {
        // Draw the leaf's share of each group, then merge the groups as a region tile merges them.
        let tile = TileGeometry::of_cell(leaf);
        for ((image, group), group_rows) in images.iter_mut().zip(groups).zip(leaf_rows) {
            let key = GroupTileKey { simulated_at_ms: group.last_update_ms.unwrap_or(0), star_count: group_rows.len(), magnitude_threshold: threshold, zoom_boost: opacities.zoom_boost() };
            image.render_rows(key, &tile, &group.stars, group.simulation.directions(), group.simulation.magnitudes(), group_rows.iter().copied(), opacities);
        }
        child.cell = leaf;
        child.image.merge_images(std::array::from_fn(|group| &images[group]));
    }
}

/// Each leaf's drawn rows of each group. A row's cell at the depth of the deepest leaf is found once and matched
/// to the leaf that contains it.
fn sort_rows_into_leaves(groups: &[StarGroup; SPEED_GROUP_COUNT], region: SkyCell, leaves: &[SkyCell], star_counts: [usize; SPEED_GROUP_COUNT], threshold: u16) -> LeafRows {
    let mut rows: LeafRows = (0..leaves.len()).map(|_| Default::default()).collect();
    let deepest = leaves.iter().map(|leaf| leaf.depth).max().unwrap_or(region.depth);
    for (group_index, (group, &star_count)) in groups.iter().zip(&star_counts).enumerate() {
        let (directions, magnitudes) = (group.simulation.directions(), group.simulation.magnitudes());
        for row in 0..star_count {
            if !is_star_drawn(magnitudes[row], threshold) { continue; }
            let Some(leaf) = find_leaf_holding(leaves, region.locate_within(directions[row], deepest)) else { continue };
            rows[leaf][group_index].push(row);
        }
    }
    rows
}

/// The position of the leaf containing `cell`, a cell at least as deep as every leaf. Leaves come in the order the
/// selection quartered them, so the cells they cover at any common depth ascend.
fn find_leaf_holding(leaves: &[SkyCell], cell: SkyCell) -> Option<usize> {
    let covering = |leaf: &SkyCell| cell.index >> (2 * (cell.depth - leaf.depth));    // the ancestor of `cell` at the leaf's depth
    let after = leaves.partition_point(|leaf| leaf.index <= covering(leaf));
    after.checked_sub(1).filter(|&position| leaves[position].index == covering(&leaves[position]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::GRID_DEPTH;
    use crate::formulas::{J2000, Observer, Vector3};
    use crate::model::{Data, ProjectionKind, View, ViewCenter, fixtures, hash_direction};
    use crate::observation::{Observation, observe};
    use crate::regions::refresh_regions;
    use crate::selection::{Selected, select_regions};
    use crate::time::StepTimes;

    /// A `fov`-degree view straight at `direction` and its observation, at J2000 from Boston.
    fn look_at(direction: Vector3, fov: f64) -> (View, Observation) {
        let (site, time) = (Observer { latitude: 0.743, longitude: -1.24 }, crate::formulas::utc_jd_to_time_ms(J2000).unwrap());
        let observe_with = |view: &View| observe(&site, false, view, time, crate::formulas::evaluate_earth_state(time), &mut StepTimes::default());
        let horizontal = observe_with(&View::default()).to_horizon.apply(direction);      // East, North, Up: the view's forward axis
        let view = View { center: ViewCenter::Facing { azimuth: horizontal.x.atan2(horizontal.y), tilt: horizontal.z.asin() }, projection: ProjectionKind::Stereographic, fov_degrees: fov };
        (view, observe_with(&view))
    }

    /// Select and refresh `data` for the view, as steps 3 and 4 do on a 1116-pixel drawing square.
    fn refresh(data: &mut Data, view: &View, observation: &Observation) -> Selected {
        let mut selected = Selected::default();
        select_regions(&mut selected, &data.star_regions, observation, view, 1116, &mut StepTimes::default());
        refresh_regions(
            &mut data.star_regions, &data.constellation_stars,
            &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
            &mut data.star_opacities, &mut data.tile_bytes,
            &selected, observation.time_ms, view.fov_degrees, 6.0, &mut StepTimes::default(),
        );
        selected
    }

    /// The child tiles holding light at `direction`'s texel.
    fn tiles_lit_at(children: &[ChildTile], direction: Vector3) -> Vec<SkyCell> {
        children.iter().filter(|child| {
            let Some((column, row)) = TileGeometry::of_cell(child.cell).project(direction) else { return false };
            let (column, row) = (column.round() as i64, row.round() as i64);
            let size = TileGeometry::size() as i64;
            (0..size).contains(&column) && (0..size).contains(&row) && !child.image.texels().is_empty() && !child.image.texels()[(row * size + column) as usize].is_empty()
        }).map(|child| child.cell).collect()
    }

    #[test]
    fn every_star_of_a_split_region_lands_in_the_one_leaf_that_holds_it() {
        // Nine stars spread over the middle of one region, in two groups.
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: 0.5 });
        let tile = TileGeometry::new(region);
        let middle = TileGeometry::middle();
        let directions: Vec<Vector3> = [-11.3, 0.7, 12.1].into_iter().flat_map(|x| [-12.2, 0.6, 11.7].map(|y| tile.direction_of(middle + x, middle + y))).collect();      // off the cell borders: stored directions are f32
        let placed: Vec<_> = directions.iter().enumerate().map(|(index, &direction)| (region, index % 2, fixtures::row(direction, 0.0, 3.0, index as u32 + 1))).collect();
        let mut data = fixtures::layout(&placed);

        // A 4° view splits the region; every star is drawn once, by the leaf holding it.
        let (view, observation) = look_at(tile.center(), 4.0);
        let selected = refresh(&mut data, &view, &observation);
        let leaves: Vec<SkyCell> = selected.regions_with_leaves().find(|&(selected, _)| selected == region).unwrap().1.to_vec();
        assert!(!leaves.is_empty() && data.star_regions[region].child_tiles.iter().map(|child| child.cell).eq(leaves.iter().copied()));
        let children = &data.star_regions[region].child_tiles;
        for &direction in &directions {
            let lit = tiles_lit_at(children, direction);
            assert_eq!(lit.len(), 1, "{direction:?} lit in {lit:?}");
            assert_eq!(lit[0], SkyCell::region(region).locate_within(direction, lit[0].depth));
        }
        let holding: std::collections::BTreeSet<SkyCell> = directions.iter().filter_map(|&direction| leaves.iter().copied().find(|&leaf| SkyCell::region(region).locate_within(direction, leaf.depth) == leaf)).collect();
        assert_eq!(children.iter().filter(|child| !child.image.is_empty() && child.image.lit_rows().iter().any(|&row| row != 0)).count(), holding.len());

        // A wide view draws the region's own tile again and releases the child tiles.
        let (view, observation) = look_at(tile.center(), 90.0);
        refresh(&mut data, &view, &observation);
        assert!(data.star_regions[region].child_tiles.is_empty() && !data.star_regions[region].merged_tile.is_empty());
    }

    #[test]
    fn a_star_drifted_out_of_its_region_goes_to_the_nearest_leaf_or_is_lost() {
        // Two stars filed in one region: one just past its edge, one far past it.
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: 0.5 });
        let tile = TileGeometry::new(region);
        let along = |texels: f64| tile.direction_of(TileGeometry::middle() + texels, TileGeometry::middle());
        let edge = (0..300).map(|step| step as f64 * 0.1).find(|&texels| hash_direction(GRID_DEPTH, along(texels)) != region).unwrap();
        let (near, far) = (along(edge + 0.3), along(edge + 5.0));
        assert!(hash_direction(GRID_DEPTH, near) != region && hash_direction(GRID_DEPTH, far) != region);
        let mut data = fixtures::layout(&[(region, 0, fixtures::row(near, 0.0, 3.0, 1)), (region, 0, fixtures::row(far, 0.0, 3.0, 2))]);

        // Aim between the region's middle and the stars so the edge leaves are in view.
        let (view, observation) = look_at(along(edge * 0.7), 4.0);
        let selected = refresh(&mut data, &view, &observation);
        assert!(selected.regions_with_leaves().any(|(selected, leaves)| selected == region && !leaves.is_empty()));
        let children = &data.star_regions[region].child_tiles;
        let lit = tiles_lit_at(children, near);
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert_eq!(lit[0], SkyCell::region(region).locate_within(near, lit[0].depth));     // the edge leaf nearest to it
        assert!(tiles_lit_at(children, far).is_empty());
    }

    #[test]
    fn cells_find_the_leaf_that_contains_them_or_none() {
        // Region 9 split into quarter 0 (whole), quarter 2 split again (its quarters 1 and 3 in view), quarter 3 whole.
        let region = SkyCell::region(9);
        let [first, _, third, fourth] = region.children();
        let leaves = [first, third.children()[1], third.children()[3], fourth];
        let deepest = GRID_DEPTH + 3;
        for (leaf_position, &leaf) in leaves.iter().enumerate() {
            for offset in 0..1 << (2 * (deepest - leaf.depth)) {
                let cell = SkyCell { depth: deepest, index: (leaf.index << (2 * (deepest - leaf.depth))) + offset };
                assert_eq!(find_leaf_holding(&leaves, cell), Some(leaf_position));
            }
        }
        for missing in [region.children()[1], third.children()[0], third.children()[2]] {
            let cell = SkyCell { depth: deepest, index: missing.index << (2 * (deepest - missing.depth)) };
            assert_eq!(find_leaf_holding(&leaves, cell), None, "{missing:?}");
        }
        assert_eq!(find_leaf_holding(&[], SkyCell { depth: deepest, index: 0 }), None);
    }
}
