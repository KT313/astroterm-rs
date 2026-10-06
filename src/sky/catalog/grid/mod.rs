//! Conservative region and brightness selection over immutable grid data.
use crate::model::{StarStorage, SkyRegion, QUANTIZATION_MARGIN};
use crate::astro::models::stars::ALWAYS_CHECKED_ANGLE;
use std::f64::consts::PI;
use crate::model::{
    ABERRATION_MARGIN, CELL_COUNT, GRID_DEPTH, REFRACTION_MARGIN, SelectionStats, SkyGrid, SelectedRegion,
};
const NUMERIC_SLACK: f64 = 1e-10;

pub(crate) fn stored_cell(stars: &StarStorage, index: usize) -> usize {
    cell_for(stars.motion_bound(index), stars.stored_direction(index))
}

/// Fast movers share the always-checked cell; everything else hashes its stored direction.
fn cell_for(motion_bound: f64, direction: crate::astro::Vector3) -> usize {
    if motion_bound > ALWAYS_CHECKED_ANGLE {
        return CELL_COUNT;
    }
    crate::model::hash_direction(GRID_DEPTH, direction)
}

/// Build cell offsets and conservative caps from the prepared star order.
pub(crate) fn build_grid(stars: &StarStorage) -> SkyGrid {
    let mut offsets = vec![0; CELL_COUNT + 1];
    for (direction, &bound) in stars.directions().outer_iter().zip(stars.motion_bounds()) {
        let direction = crate::astro::Vector3 { x: f64::from(direction[0]), y: f64::from(direction[1]), z: f64::from(direction[2]) };
        let cell = cell_for(f64::from(bound), direction);
        if cell < CELL_COUNT {
            offsets[cell + 1] += 1;
        }
    }
    for i in 1..offsets.len() {
        offsets[i] += offsets[i - 1];
    }
    SkyGrid {
        offsets: offsets.into(),
        coarse_caps: crate::model::build_caps(4),
        fine_caps: crate::model::build_caps(GRID_DEPTH),
    }
}

/// Select conservative regions, then collect the brightness-sorted cell prefixes.
pub fn select_grid(
    grid: &SkyGrid,
    stars: &StarStorage,
    region: SkyRegion,
    observer: &crate::model::ObserverState,
    threshold: f64,
    refraction: bool,
    indices: &mut Vec<usize>,
) -> SelectionStats {
    let region = select_region(grid, region, observer, refraction);
    select_brightness(grid, stars, &region, threshold, indices)
}

/// Select conservative cells; exact visibility is determined later by projection.
pub(crate) fn select_region(
    grid: &SkyGrid,
    region: SkyRegion,
    observer: &crate::model::ObserverState,
    refraction: bool,
) -> SelectedRegion {
    if !crate::astro::COMPUTATIONAL_INTERVAL.contains(observer.time.tt) {
        return SelectedRegion {
            cells: Vec::new(),
            brute_force: true,
        };
    }
    let mut cells = Vec::new();
    match region {
        SkyRegion::Cone { center, radius } if radius < 150_f64.to_radians() => {
            let margin = 0.1_f64.to_radians() / 3600.0 // intrinsic stellar-cache angular allowance
                + ALWAYS_CHECKED_ANGLE
                + QUANTIZATION_MARGIN
                + ABERRATION_MARGIN.max(
                    (observer.state.velocity.length() / crate::sky::LIGHT_SPEED_AU_DAY)
                        .clamp(0.0, 1.0)
                        .asin(),
                )
                + NUMERIC_SLACK
                + if refraction { REFRACTION_MARGIN } else { 0.0 };
            let radius = (radius + margin).min(PI);
            let center = observer.inertial_to_horizon.transpose().apply(center).normalized();
            let fine = radius < 30_f64.to_radians();
            for (parent, cap) in grid.coarse_caps.iter().enumerate() {
                if !cap.intersects(center, radius) {
                    continue;
                }
                for child in parent * 16..(parent + 1) * 16 {
                    if !fine || grid.fine_caps[child].intersects(center, radius) {
                        cells.push(child);
                    }
                }
            }
        }
        _ => {
            for cell in 0..CELL_COUNT {
                cells.push(cell);
            }
        }
    }
    SelectedRegion {
        cells,
        brute_force: false,
    }
}

/// Count region membership without expanding the sorted catalog ranges.
pub(crate) fn count_region_stars(grid: &SkyGrid, region: &SelectedRegion, total: usize) -> (usize, usize, usize) {
    let always = total - grid.offsets[CELL_COUNT];
    if region.brute_force {
        return (CELL_COUNT, total, always);
    }
    let count = region
        .cells
        .iter()
        .map(|&cell| grid.offsets[cell + 1] - grid.offsets[cell])
        .sum::<usize>();
    (region.cells.len(), count + always, always)
}

/// Collect stars satisfying interval-wide brightness bounds, including the always-checked tail.
pub(crate) fn select_brightness(
    grid: &SkyGrid,
    stars: &StarStorage,
    region: &SelectedRegion,
    threshold: f64,
    indices: &mut Vec<usize>,
) -> SelectionStats {
    indices.clear();
    let keys = stars.brightness_keys();
    if region.brute_force {
        indices.extend(0..stars.len());
    } else {
        for &cell in &region.cells {
            append_bright(keys, grid.offsets[cell]..grid.offsets[cell + 1], threshold, indices);
        }
        append_bright(keys, grid.offsets[CELL_COUNT]..stars.len(), threshold, indices);
    }
    SelectionStats {
        cells: if region.brute_force {
            CELL_COUNT
        } else {
            region.cells.len()
        },
        candidates: indices.len(),
        brute_force: region.brute_force,
    }
}

fn append_bright(keys: &[f32], range: std::ops::Range<usize>, threshold: f64, indices: &mut Vec<usize>) {
    for i in range {
        if f64::from(keys[i]) > threshold {
            break;
        }
        indices.push(i);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{build_caps, hash_direction};
    use crate::astro::Vector3;
    use crate::model::{interleave, direction};
    use super::*;
    use crate::astro::{Equatorial, Horizontal, apply_refraction};
    use proptest::prelude::*;

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
            let coarse = build_caps(4);
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
