//! Fixed catalog-epoch regions and conservative brightness selection. Motion beyond the fixed margin may be culled.
use crate::constants::{CONSTELLATION_REGION, GRID_DEPTH, SIMULATION_REGION_COUNT};
use crate::model::StarStorage;
use crate::model::SkyGrid;

/// Geometric cell for a stored direction; constellation endpoints instead belong to the dedicated region.
pub(crate) fn stored_cell(stars: &StarStorage, index: usize) -> usize {
    crate::model::hash_direction(GRID_DEPTH, stars.stored_direction(index))
}

/// Build region boundaries covering every row of the prepared star order.
#[cfg(test)]
pub(crate) fn build_grid(stars: &StarStorage, endpoints: &[usize], progress: &mut dyn crate::progress::DatasetProgress) -> SkyGrid {
    let mut offsets = vec![0; SIMULATION_REGION_COUNT + 1];
    let ordinary_count = stars.len() - endpoints.len();
    assert!(endpoints.iter().copied().eq(ordinary_count..stars.len()), "constellation region must contain exactly the endpoints");
    offsets[CONSTELLATION_REGION + 1] = endpoints.len();
    for index in 0..ordinary_count {
        offsets[stored_cell(stars, index) + 1] += 1;
        if (index + 1).is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS as usize) { progress.set_position((index + 1) as u64); }
    }
    for i in 1..offsets.len() {
        offsets[i] += offsets[i - 1];
    }
    progress.set_position(stars.len() as u64);
    progress.set_detail("Preparing region coverage");
    SkyGrid::from_offsets(offsets.into())
}

/// Reuse counts from region assignment; no second direction-hashing pass is needed.
pub(super) fn build_grid_from_counts(stars: usize, mut counts: Vec<usize>, endpoints: &[usize], progress: &mut dyn crate::progress::DatasetProgress) -> std::io::Result<SkyGrid> {
    use crate::catalog::cache::invalid;
    if counts.len() != SIMULATION_REGION_COUNT + 1 || counts[0] != 0 || counts[CONSTELLATION_REGION + 1] != endpoints.len() {
        return Err(invalid("invalid prepared region counts"));
    }
    for i in 1..counts.len() { counts[i] = counts[i].checked_add(counts[i - 1]).ok_or_else(|| invalid("region count overflow"))?; }
    if counts[SIMULATION_REGION_COUNT] != stars || !endpoints.iter().copied().eq(counts[CONSTELLATION_REGION]..stars) {
        return Err(invalid("region counts differ from prepared rows/endpoints"));
    }
    progress.set_position(stars as u64);
    progress.set_detail("Preparing region coverage");
    Ok(SkyGrid::from_offsets(counts.into()))
}

#[cfg(test)]
mod tests {
    use crate::model::{build_caps, hash_direction};
    use crate::astro::Vector3;
    use crate::model::{interleave, direction};
    use crate::astro::{Equatorial, Horizontal, apply_refraction};
    use crate::constants::REFRACTION_MARGIN;
    use proptest::prelude::*;
    use std::f64::consts::PI;

    #[test]
    fn refraction_margin_covers_the_clamped_formula() {
        let mut maximum = 0.0_f64;
        for i in -9000..=9000 {
            let altitude = (i as f64 / 100.0).to_radians();
            maximum = maximum.max(apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude - altitude);
        }
        assert!(maximum <= REFRACTION_MARGIN);
        assert!(maximum.to_degrees() > 0.646);
    }

    #[test]
    fn caps_cover_vertices_edges_and_nested_children_including_face_seams() {
        for depth in [4, 6] {
            let caps = build_caps(depth);
            let coarse = build_caps(crate::constants::GRID_COARSE_DEPTH);
            let n = 1_usize << depth;
            for face in 0..6 {
                for x in 0..n {
                    for y in 0..n {
                        let cell = face * n * n + interleave(x, y, depth);
                        for dx in [0.0, 0.5, 1.0] {
                            for dy in [0.0, 0.5, 1.0] {
                                let p = direction(
                                    face,
                                    -1.0 + 2.0 * (x as f64 + dx) / n as f64,
                                    -1.0 + 2.0 * (y as f64 + dy) / n as f64,
                                );
                                assert!(caps[cell].intersects(p, 0.0));
                                assert!(coarse[cell >> (2 * (depth - 4))].intersects(p, 0.0));
                                assert!(caps[hash_direction(depth, p)].intersects(p, 0.0));
                            }
                        }
                    }
                }
            }
        }
    }

    proptest! {
        #[test]
        fn cone_bounds_cover_interior_and_edge_points(
            lon in 0.0_f64..(2.0*PI), lat in -PI/2.0..PI/2.0,
            radius in 1e-7_f64..PI, bearing in 0.0_f64..(2.0*PI), fraction in 0.0_f64..1.0,
        ) {
            let center = Equatorial { right_ascension: lon, declination: lat }.to_unit_vector();
            let east = Vector3 { x: -lon.sin(), y: lon.cos(), z: 0.0 };
            let north = center.cross(east);
            for depth in [4,6] {
                let caps = build_caps(depth);
                for factor in [0.0, fraction, 1.0 - 1e-12, 1.0] {
                    let angle = radius * factor;
                    let point = center * angle.cos() + (east * bearing.cos() + north * bearing.sin()) * angle.sin();
                    prop_assert!(caps[hash_direction(depth,point)].intersects(center,radius));
                }
            }
        }
    }
}
