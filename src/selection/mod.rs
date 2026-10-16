//! Step 3 of the frame: `select_regions`.
//!
//! Inputs: the star tables and the observation. Output: the regions whose stars can appear in the view, in
//! ascending order, and whether that is the whole sky. The constellation table is always part of the selection
//! and is not listed. Runs every frame and is cheap: at most the 1,536 coarse caps and the fine caps of the
//! coarse cells that intersect the patch; empty regions are never selected.
//!
//! The patch already carries the margins (drift, aberration, refraction, quantization), so a region is selected
//! when its bounding cap intersects the patch cone. Outside the computational interval, or when the patch covers
//! 150° or more, every non-empty region is selected, as the old selection did.
use crate::time::StepTimes;
use std::sync::OnceLock;

use crate::formulas::COMPUTATIONAL_INTERVAL;
use crate::constants::{CELL_COUNT, GRID_CHILDREN_PER_COARSE_CELL, GRID_COARSE_DEPTH};
use crate::model::{CellCap, Region, build_caps, region_caps};
use crate::observation::SkyPatch;
#[cfg(test)]
use crate::observation::Observation;

/// The regions a frame works on. The constellation table is always included and is not listed here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub regions: Vec<usize>,                // ascending region numbers, all non-empty
    pub whole_sky: bool,                    // every non-empty region is listed: a wide view or a time outside the computational interval
}

/// The regions whose caps intersect the observed patch, or every non-empty region for a wide view.
pub fn select_regions(stars: &[Region], patch: &SkyPatch, time_ms: i64, times: &mut StepTimes) -> Selected {
    let whole_sky = !COMPUTATIONAL_INTERVAL.contains(crate::formulas::AstronomicalTime::from_time_ms(time_ms).tt_jd) || patch.radius >= 150_f64.to_radians();
    let has_stars = |region: usize| stars[region].speed_groups.iter().any(|group| !group.stars.is_empty());
    let mut regions = Vec::new();
    if whole_sky {
        times.measure("Selection.Populated regions");
        regions.extend((0..CELL_COUNT).filter(|&region| has_stars(region)));
        return Selected { regions, whole_sky };
    }

    // coarse caps first, fine caps only for narrow patches
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
    Selected { regions, whole_sky }
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
    use crate::model::{ProjectionKind, Data, View, ViewCenter, hash_direction};
    use crate::observation::observe;

    fn bsc5() -> Data {
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut crate::progress::Report::quiet()).unwrap(), &mut crate::progress::Report::quiet()).unwrap();
        crate::sky::build_sky_tables(&prepared, &mut crate::progress::Report::quiet()).unwrap()
    }
    fn boston() -> Observer { Observer { latitude: 42.3601_f64.to_radians(), longitude: -71.0589_f64.to_radians() } }
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
    fn empty_tables_select_nothing() {
        let observation = observe(&boston(), false, &facing(360.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
        let selected = select_regions(&Data::default().star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
        assert_eq!(selected, Selected { regions: Vec::new(), whole_sky: true });
    }

    #[test]
    fn narrow_views_select_few_regions_that_cover_the_patch() {
        let tables = bsc5();
        let observation = observe(&boston(), true, &facing(5.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
        let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
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
            let observation = observe(&boston(), false, &facing(fov), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
            let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
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
        let observation = observe(&boston(), true, &facing(300.0), crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
        let wide = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
        assert_eq!(wide, Selected { regions: filled.clone(), whole_sky: true });
        let outside = crate::formulas::utc_jd_to_time_ms(COMPUTATIONAL_INTERVAL.end_tt + 1.0).unwrap();
        let observation = observe(&boston(), false, &facing(1.0), outside, &mut crate::time::StepTimes::default());
        let late = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
        assert_eq!(late, Selected { regions: filled, whole_sky: true });
    }

    #[test]
    fn the_region_of_the_view_centre_is_always_selected() {
        let tables = bsc5();
        for (azimuth, tilt) in [(0.0, 0.1), (1.0, 1.2), (3.0, -0.4), (5.5, 0.9)] {
            let view = View { center: ViewCenter::Facing { azimuth, tilt }, ..facing(2.0) };
            let observation = observe(&boston(), false, &view, crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), &mut crate::time::StepTimes::default());
            let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut crate::time::StepTimes::default());
            let centre = hash_direction(GRID_DEPTH, observation.patch.center);
            let holds_stars = tables.star_regions[centre].speed_groups.iter().map(|group| &group.stars).any(|table| !table.is_empty());
            assert_eq!(selected.regions.binary_search(&centre).is_ok(), holds_stars, "{azimuth} {tilt}");
        }
    }
}
