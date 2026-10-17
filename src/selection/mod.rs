//! Step 3 of the frame: `select_regions`.
//!
//! Inputs: the star tables, the observation, the view and the drawing square's size. Output: the regions whose
//! stars can appear in the view, in ascending order, whether that is the whole sky, and the finer cells that
//! replace the tiles of zoomed-in regions. The constellation table is always part of the selection and is not
//! listed. Runs every frame and is cheap: at most the 1,536 coarse caps and the fine caps of the coarse cells
//! that intersect the patch; empty regions are never selected.
//!
//! The patch already carries the margins (drift, aberration, refraction, quantization), so a region is selected
//! when its bounding cap intersects the patch cone. Outside the computational interval, or when the patch covers
//! 150° or more, every non-empty region is selected, as the old selection did.
//!
//! A selected region whose tile would show `MIN_TILE_TEXELS_PER_PIXEL` texels per screen pixel or fewer is split
//! (`leaves`): its tile is replaced by the tiles of the in-view quarters of its cell, quartered again until each
//! is fine enough (`tile_scale` estimates the ratio). Outside the computational interval no region splits, because
//! stars may lie far from their regions there.
use crate::time::StepTimes;
use std::sync::OnceLock;

use crate::formulas::{AstronomicalTime, COMPUTATIONAL_INTERVAL};
use crate::constants::{CELL_COUNT, GRID_CHILDREN_PER_COARSE_CELL, GRID_COARSE_DEPTH};
use crate::model::{CellCap, Region, SkyCell, View, build_caps, region_caps};
use crate::observation::{Observation, SkyPatch};
mod leaves;
mod tile_scale;
use leaves::{is_fine_enough, split_into_leaves};
pub use tile_scale::{TileScale, estimate_texels_per_pixel};

/// The regions a frame works on. The constellation table is always included and is not listed here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    pub regions: Vec<usize>,                // ascending region numbers, all non-empty
    pub whole_sky: bool,                    // every non-empty region is listed: a wide view or a time outside the computational interval
    pub leaves: Vec<SkyCell>,               // the cells drawn instead of split regions' tiles, grouped by region in region order
}

impl Selected {
    /// A selection of `regions` (ascending) that draws each of them as it is, splitting none.
    pub fn of_regions(regions: Vec<usize>) -> Self { Self { regions, whole_sky: false, leaves: Vec::new() } }

    /// Each selected region with the cells that replace its tile, in region order: none when it is drawn as it is.
    pub fn regions_with_leaves(&self) -> impl Iterator<Item = (usize, &[SkyCell])> {
        let mut rest = self.leaves.as_slice();
        self.regions.iter().map(move |&region| {
            let (leaves, tail) = rest.split_at(rest.iter().take_while(|leaf| leaf.region_index() == region).count());
            rest = tail;
            (region, leaves)
        })
    }
}

/// Refill the retained selection with intersecting regions, or all non-empty regions for a wide view, and split
/// those that would look too coarse on a drawing square `size` pixels wide.
pub fn select_regions(selected: &mut Selected, stars: &[Region], observation: &Observation, view: &View, size: usize, times: &mut StepTimes) {
    // Decide whether caps can narrow the selection at this simulation time and field of view.
    let patch = &observation.patch;
    let within_interval = COMPUTATIONAL_INTERVAL.contains(AstronomicalTime::from_time_ms(observation.time_ms).tt_jd);
    let whole_sky = !within_interval || patch.radius >= 150_f64.to_radians();
    let has_stars = |region: usize| stars[region].speed_groups.iter().any(|group| !group.stars.is_empty());

    // Keep the region-index allocation while replacing the previous frame's selection.
    selected.whole_sky = whole_sky;
    let regions = &mut selected.regions;
    regions.clear();
    if whole_sky {
        times.measure("Selection.Populated regions");
        regions.extend((0..CELL_COUNT).filter(|&region| has_stars(region)));
    } else {
        // Check coarse sky caps first, then individual region caps for narrow views.
        times.measure("Selection.Region caps");
        let caps = caps();
        let fine = patch.radius < 30_f64.to_radians();
        times.measure("Selection.Cap intersection filtering");
        for (parent, cap) in caps.coarse.iter().enumerate() {
            if !cap.intersects(patch.center, patch.radius) { continue; }
            for child in parent * GRID_CHILDREN_PER_COARSE_CELL..(parent + 1) * GRID_CHILDREN_PER_COARSE_CELL {
                if (!fine || caps.fine[child].intersects(patch.center, patch.radius)) && has_stars(child) { regions.push(child); }
            }
        }
    }

    // Split the regions whose tiles would be too coarse on screen.
    times.measure("Selection.Region splits");
    selected.leaves.clear();
    if within_interval { split_coarse_regions(selected, &TileScale::new(observation, view, size), patch); }
}

/// Fill `selected.leaves` with the cells that replace the tiles of selected regions too coarse on screen. A region
/// out of the patch (a whole-sky selection) has no quarter in it and stays as it is.
fn split_coarse_regions(selected: &mut Selected, scale: &TileScale, patch: &SkyPatch) {
    let cells: Vec<SkyCell> = selected.regions.iter().map(|&region| SkyCell::region(region)).collect();
    for (region, texels_per_pixel) in estimate_texels_per_pixel(&cells, scale) {
        if is_fine_enough(region, texels_per_pixel) || !region.cap().intersects(patch.center, patch.radius) { continue; }
        split_into_leaves(region, scale, patch, &mut selected.leaves);
    }
}

/// The bounding caps of the coarse and the fine grid cells: fixed geometry, built on first use.
struct RegionCaps {
    coarse: Vec<CellCap>,
    fine: &'static [CellCap],
}

fn caps() -> &'static RegionCaps {
    static CAPS: OnceLock<RegionCaps> = OnceLock::new();
    CAPS.get_or_init(|| RegionCaps { coarse: build_caps(GRID_COARSE_DEPTH), fine: region_caps() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::{J2000, Observer};
    use crate::constants::GRID_DEPTH;
    use crate::constants::{MAX_TILE_SPLITS, MIN_TILE_TEXELS_PER_PIXEL};
    use crate::formulas::Vector3;
    use crate::model::{ProjectionKind, Data, ViewCenter, hash_direction};
    use crate::observation::observe;

    fn bsc5() -> Data {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap(), &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
    const SIZE: usize = 1400;                                                                // the drawing square of a 2400 × 1400 frame
    fn facing(fov_degrees: f64) -> View { View { center: ViewCenter::Facing { azimuth: 2.0, tilt: 0.5 }, projection: ProjectionKind::Stereographic, fov_degrees } }

    /// Every region that holds a star inside the patch cone must be selected.
    fn check_covers_the_patch(tables: &Data, selected: &Selected, observation: &Observation) {
        let patch = observation.patch;
        for (region, _, table) in tables.star_tables() {
            for row in 0..table.len() {
                if table.stored_direction(row).dot(patch.center) >= patch.radius.cos() {
                    assert!(selected.regions.binary_search(&region).is_ok(), "region {region} holds star {} inside the patch", table.id(row).0);
                }
            }
        }
    }

    #[test]
    fn selection_reuses_storage_and_replaces_stale_indices_and_whole_sky_flag() {
        let data = bsc5();
        let time = crate::formulas::utc_jd_to_time_ms(J2000).unwrap();
        let earth = crate::formulas::evaluate_earth_state(time);
        let mut selected = Selected::default();
        let mut times = crate::time::StepTimes::default();
        let wide = observe(&boston(), false, &facing(359.0), time, earth, &mut times);
        select_regions(&mut selected, &data.star_regions, &wide, &facing(359.0), SIZE, &mut times);
        assert!(selected.whole_sky);
        let address = selected.regions.as_ptr();
        let capacity = selected.regions.capacity();

        // A narrower view refills the same storage and must not retain the whole-sky flag or indices.
        let narrow = observe(&boston(), false, &facing(10.0), time, earth, &mut times);
        let mut expected = Selected::default();
        select_regions(&mut expected, &data.star_regions, &narrow, &facing(10.0), SIZE, &mut times);
        select_regions(&mut selected, &data.star_regions, &narrow, &facing(10.0), SIZE, &mut times);
        assert_eq!(selected, expected);
        assert!(!selected.whole_sky);
        assert_eq!(selected.regions.as_ptr(), address);
        assert_eq!(selected.regions.capacity(), capacity);

        // An empty catalog clears the remaining indices while retaining their allocation.
        select_regions(&mut selected, &Data::default().star_regions, &wide, &facing(359.0), SIZE, &mut times);
        assert!(selected.regions.is_empty());
        assert!(selected.whole_sky);
        assert_eq!(selected.regions.as_ptr(), address);
    }

    #[test]
    fn empty_tables_select_nothing() {
        let observation = observe(&boston(), false, &facing(360.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
        let mut selected = crate::selection::Selected::default();
        select_regions(&mut selected, &Data::default().star_regions, &observation, &facing(360.0), SIZE, &mut crate::time::StepTimes::default());
        assert_eq!(selected, Selected { regions: Vec::new(), whole_sky: true, leaves: Vec::new() });
    }

    #[test]
    fn narrow_views_select_few_regions_that_cover_the_patch() {
        let tables = bsc5();
        let observation = observe(&boston(), true, &facing(5.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
        let mut selected = crate::selection::Selected::default();
        select_regions(&mut selected, &tables.star_regions, &observation, &facing(5.0), SIZE, &mut crate::time::StepTimes::default());
        assert!(!selected.whole_sky);
        assert!(!selected.regions.is_empty() && selected.regions.len() < 60, "{}", selected.regions.len());
        assert!(selected.regions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(selected.regions.iter().all(|&region| region < CELL_COUNT && tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).any(|table| !table.is_empty())));
        check_covers_the_patch(&tables, &selected, &observation);
    }

    #[test]
    fn medium_views_use_coarse_caps_and_cover_the_patch() {
        let tables = bsc5();
        for fov in [60.0, 120.0, 180.0, 240.0] {
            let observation = observe(&boston(), false, &facing(fov), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
            let mut selected = crate::selection::Selected::default();
            select_regions(&mut selected, &tables.star_regions, &observation, &facing(fov), SIZE, &mut crate::time::StepTimes::default());
            assert!(!selected.whole_sky, "{fov}");
            check_covers_the_patch(&tables, &selected, &observation);
            let parents: std::collections::BTreeSet<_> = selected.regions.iter().map(|region| region / GRID_CHILDREN_PER_COARSE_CELL).collect();
            assert!(parents.iter().all(|&parent| caps().coarse[parent].intersects(observation.patch.center, observation.patch.radius)));
        }
    }

    #[test]
    fn wide_views_and_times_outside_the_interval_select_the_whole_sky() {
        let tables = bsc5();
        let filled: Vec<_> = (0..CELL_COUNT).filter(|&region| tables.star_regions[region].speed_groups.iter().map(|group| &group.stars).any(|table| !table.is_empty())).collect();
        let observation = observe(&boston(), true, &facing(300.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
        let mut wide = crate::selection::Selected::default();
        select_regions(&mut wide, &tables.star_regions, &observation, &facing(300.0), SIZE, &mut crate::time::StepTimes::default());
        assert_eq!(wide, Selected { regions: filled.clone(), whole_sky: true, leaves: Vec::new() });
        let outside = crate::formulas::utc_jd_to_time_ms(COMPUTATIONAL_INTERVAL.end_tt + 1.0).unwrap();
        let observation = observe(&boston(), false, &facing(1.0), outside, crate::formulas::evaluate_earth_state(outside), &mut crate::time::StepTimes::default());
        let mut late = crate::selection::Selected::default();
        select_regions(&mut late, &tables.star_regions, &observation, &facing(1.0), SIZE, &mut crate::time::StepTimes::default());
        assert_eq!(late, Selected { regions: filled, whole_sky: true, leaves: Vec::new() });                    // no splits outside the interval
    }

    #[test]
    fn the_region_of_the_view_centre_is_always_selected() {
        let tables = bsc5();
        for (azimuth, tilt) in [(0.0, 0.1), (1.0, 1.2), (3.0, -0.4), (5.5, 0.9)] {
            let view = View { center: ViewCenter::Facing { azimuth, tilt }, ..facing(2.0) };
            let observation = observe(&boston(), false, &view, crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
            let mut selected = crate::selection::Selected::default();
            select_regions(&mut selected, &tables.star_regions, &observation, &view, SIZE, &mut crate::time::StepTimes::default());
            let centre = hash_direction(GRID_DEPTH, observation.patch.center);
            let holds_stars = tables.star_regions[centre].speed_groups.iter().map(|group| &group.stars).any(|table| !table.is_empty());
            assert_eq!(selected.regions.binary_search(&centre).is_ok(), holds_stars, "{azimuth} {tilt}");
        }
    }

    #[test]
    fn no_region_splits_at_wide_views() {
        let tables = bsc5();
        for fov in [90.0, 180.0, 300.0] {
            let observation = observe(&boston(), false, &facing(fov), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
            let mut selected = crate::selection::Selected::default();
            select_regions(&mut selected, &tables.star_regions, &observation, &facing(fov), SIZE, &mut crate::time::StepTimes::default());
            assert!(selected.leaves.is_empty(), "{fov}: {} leaves", selected.leaves.len());
            assert!(selected.regions_with_leaves().all(|(_, leaves)| leaves.is_empty()));
        }
    }

    #[test]
    fn narrow_views_split_regions_into_fine_cells_that_cover_the_view() {
        let tables = bsc5();
        for (fov, projection) in [(20.0, ProjectionKind::Stereographic), (5.0, ProjectionKind::Equidistant), (1.0, ProjectionKind::Stereographic)] {
            let view = View { projection, ..facing(fov) };
            let observation = observe(&boston(), true, &view, crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
            let mut selected = crate::selection::Selected::default();
            select_regions(&mut selected, &tables.star_regions, &observation, &view, SIZE, &mut crate::time::StepTimes::default());
            let scale = TileScale::new(&observation, &view, SIZE);
            let patch = observation.patch;
            assert!(!selected.leaves.is_empty(), "fov={fov}");

            // Every region is listed once, with leaves exactly when its own tile is too coarse and it reaches into the patch.
            let split: Vec<(usize, &[SkyCell])> = selected.regions_with_leaves().collect();
            assert_eq!(split.iter().map(|&(region, _)| region).collect::<Vec<_>>(), selected.regions);
            assert_eq!(split.iter().map(|(_, leaves)| leaves.len()).sum::<usize>(), selected.leaves.len());
            for &(region, leaves) in &split {
                let cell = SkyCell::region(region);
                if scale.texels_per_pixel(cell) > MIN_TILE_TEXELS_PER_PIXEL { assert!(leaves.is_empty(), "fov={fov} region {region}"); }

                // Leaves are fine enough or at the depth limit, in view, and never inside one another.
                for (position, &leaf) in leaves.iter().enumerate() {
                    assert_eq!(leaf.region_index(), region);
                    assert!(leaf.depth > GRID_DEPTH && (leaf.depth == GRID_DEPTH + MAX_TILE_SPLITS || scale.texels_per_pixel(leaf) > MIN_TILE_TEXELS_PER_PIXEL), "fov={fov} {leaf:?}");
                    assert!(leaf.cap().intersects(patch.center, patch.radius), "fov={fov} {leaf:?}");
                    for &other in &leaves[position + 1..] {
                        let (shallow, deep) = if leaf.depth <= other.depth { (leaf, other) } else { (other, leaf) };
                        assert_ne!(deep.index >> (2 * (deep.depth - shallow.depth)), shallow.index, "fov={fov}: {deep:?} inside {shallow:?}");
                    }
                }
            }

            // Every direction inside the patch lies in a leaf of its region when that region is split.
            let (east, north) = (Vector3 { x: 0.0, y: 0.0, z: 1.0 }.cross(patch.center).normalized(), patch.center.cross(Vector3 { x: 0.0, y: 0.0, z: 1.0 }.cross(patch.center).normalized()));
            for step in 0..400 {
                let (radius, angle) = (patch.radius * ((step % 20) as f64 / 20.0), std::f64::consts::TAU * (step / 20) as f64 / 20.0);
                let direction = (patch.center * radius.cos() + (east * angle.cos() + north * angle.sin()) * radius.sin()).normalized();
                let region = hash_direction(GRID_DEPTH, direction);
                let Some(&(_, leaves)) = split.iter().find(|&&(selected, _)| selected == region) else { continue };
                if leaves.is_empty() { continue; }
                assert!(leaves.iter().any(|&leaf| SkyCell::region(region).locate_within(direction, leaf.depth) == leaf), "fov={fov}: {direction:?} in no leaf");
            }
        }
    }

    #[test]
    #[ignore = "overview only: prints how far regions split at narrow views and how long the selection takes"]
    fn print_split_overview() {
        let tables = bsc5();
        for fov in [180.0, 90.0, 20.0, 10.0, 5.0, 1.0] {
            let observation = observe(&boston(), false, &facing(fov), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), crate::formulas::evaluate_earth_state(crate::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut crate::time::StepTimes::default());
            let mut selected = crate::selection::Selected::default();
            let runs = 200;
            let start = std::time::Instant::now();
            for _ in 0..runs { select_regions(&mut selected, &tables.star_regions, &observation, &facing(fov), SIZE, &mut crate::time::StepTimes::default()); }
            let elapsed = start.elapsed().as_secs_f64() * 1e6 / runs as f64;
            let split = selected.regions_with_leaves().filter(|(_, leaves)| !leaves.is_empty()).count();
            let depths: std::collections::BTreeMap<u8, usize> = selected.leaves.iter().fold(Default::default(), |mut counts, leaf| { *counts.entry(leaf.depth - GRID_DEPTH).or_default() += 1; counts });
            eprintln!("fov={fov:>5}: {} regions, {split} split into {} leaves, splits per depth {depths:?}, {elapsed:.1} µs per selection", selected.regions.len(), selected.leaves.len());
        }
    }
}
