//! Conservative cube-map selection. Each gnomonic cell is contained in the spherical cap covering its four
//! corners: caps smaller than a hemisphere are geodesically convex. Morton order makes each depth-4 cell's
//! sixteen depth-6 children contiguous. Exact current-position visibility remains projection's responsibility.
use super::{StarStorage, storage::QUANTIZATION_MARGIN};
use crate::astro::{Vector3, models::stars::ALWAYS_CHECKED_ANGLE};
use std::f64::consts::PI;

pub const GRID_DEPTH: u8 = 6;
pub const CELL_COUNT: usize = 6 << (2 * GRID_DEPTH);
pub const REFRACTION_MARGIN: f64 = 0.647 * PI / 180.0;
/// Qualified against 200,001 Earth-velocity samples plus maximum WGS84 site spin (21.219703″).
/// Selection also expands this from the actual observer velocity, independently of the sampled bound.
pub const ABERRATION_MARGIN: f64 = 22.0 * PI / (180.0 * 3600.0);
const NUMERIC_SLACK: f64 = 1e-10;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SkyRegion {
    #[default]
    All,
    /// Unit center in the observer's East/North/Up frame, angular radius in radians.
    Cone { center: Vector3, radius: f64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionStats {
    pub cells: usize,
    pub candidates: usize,
    pub brute_force: bool,
}

/// Intermediate region selection, consumed by the independently timed brightness pass.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectedRegion {
    cells: Vec<usize>,
    brute_force: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyGrid {
    pub offsets: crate::catalog::cache::CatalogArray<usize>,
    coarse_caps: Vec<CellCap>,
    fine_caps: Vec<CellCap>,
}

pub(crate) fn stored_cell(stars: &StarStorage, index: usize) -> usize {
    if stars.motion_bound(index) > ALWAYS_CHECKED_ANGLE {
        return CELL_COUNT;
    }
    hash_direction(GRID_DEPTH, stars.stored_direction(index))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CellCap {
    center: Vector3,
    radius: f64,
}

impl CellCap {
    fn intersects(self, center: Vector3, radius: f64) -> bool {
        let sum = radius + self.radius + NUMERIC_SLACK;
        sum >= PI || self.center.dot(center) >= sum.cos()
    }
}

/// Assign even exact face/edge/corner ties deterministically, using the stored direction.
pub fn hash_direction(depth: u8, p: Vector3) -> usize {
    let (x, y, z) = (p.x.abs(), p.y.abs(), p.z.abs());
    let (face, u, v) = if x >= y && x >= z {
        (usize::from(p.x < 0.0), p.y / x, p.z / x)
    } else if y >= z {
        (2 + usize::from(p.y < 0.0), p.x / y, p.z / y)
    } else {
        (4 + usize::from(p.z < 0.0), p.x / z, p.y / z)
    };
    let n = 1_usize << depth;
    let index = |q: f64| (((q + 1.0) * 0.5 * n as f64).floor() as usize).min(n - 1);
    face * n * n + interleave(index(u), index(v), depth)
}
fn interleave(x: usize, y: usize, depth: u8) -> usize {
    (0..depth)
        .map(|bit| ((x >> bit) & 1) << (2 * bit) | ((y >> bit) & 1) << (2 * bit + 1))
        .sum()
}
fn direction(face: usize, u: f64, v: f64) -> Vector3 {
    match face {
        0 => Vector3 { x: 1.0, y: u, z: v },
        1 => Vector3 { x: -1.0, y: u, z: v },
        2 => Vector3 { x: u, y: 1.0, z: v },
        3 => Vector3 { x: u, y: -1.0, z: v },
        4 => Vector3 { x: u, y: v, z: 1.0 },
        _ => Vector3 { x: u, y: v, z: -1.0 },
    }
    .normalized()
}
fn build_caps(depth: u8) -> Vec<CellCap> {
    let n = 1_usize << depth;
    let mut caps = vec![
        CellCap {
            center: Vector3::default(),
            radius: 0.0
        };
        6 * n * n
    ];
    for face in 0..6 {
        for x in 0..n {
            for y in 0..n {
                let (u, v) = (-1.0 + 2.0 * x as f64 / n as f64, -1.0 + 2.0 * y as f64 / n as f64);
                let step = 2.0 / n as f64;
                let center = direction(face, u + step / 2.0, v + step / 2.0);
                let radius = [(u, v), (u + step, v), (u, v + step), (u + step, v + step)]
                    .into_iter()
                    .map(|(u, v)| {
                        let corner = direction(face, u, v);
                        center.cross(corner).length().atan2(center.dot(corner))
                    })
                    .fold(0.0, f64::max)
                    .next_up();
                caps[face * n * n + interleave(x, y, depth)] = CellCap { center, radius };
            }
        }
    }
    caps
}

impl SkyGrid {
    pub(crate) fn from_offsets(offsets: crate::catalog::cache::CatalogArray<usize>) -> Self {
        Self {
            offsets,
            coarse_caps: build_caps(4),
            fine_caps: build_caps(GRID_DEPTH),
        }
    }
    pub(crate) fn build(stars: &StarStorage) -> Self {
        let mut offsets = vec![0; CELL_COUNT + 1];
        for i in 0..stars.len() {
            let cell = stored_cell(stars, i);
            if cell < CELL_COUNT {
                offsets[cell + 1] += 1;
            }
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        Self {
            offsets: offsets.into(),
            coarse_caps: build_caps(4),
            fine_caps: build_caps(GRID_DEPTH),
        }
    }

    pub fn select(
        &self,
        stars: &StarStorage,
        region: SkyRegion,
        observer: &super::ObserverState,
        threshold: f64,
        refraction: bool,
        indices: &mut Vec<usize>,
    ) -> SelectionStats {
        let region = self.select_region(region, observer, refraction);
        self.select_brightness(stars, &region, threshold, indices)
    }

    /// Select conservative cells only; brightness is a separate pass over their sorted prefixes.
    pub(crate) fn select_region(
        &self,
        region: SkyRegion,
        observer: &super::ObserverState,
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
                        (observer.state.velocity.length() / super::observation::LIGHT_SPEED_AU_DAY)
                            .clamp(0.0, 1.0)
                            .asin(),
                    )
                    + NUMERIC_SLACK
                    + if refraction { REFRACTION_MARGIN } else { 0.0 };
                let radius = (radius + margin).min(PI);
                let center = observer.inertial_to_horizon.transpose().apply(center).normalized();
                let fine = radius < 30_f64.to_radians();
                for (parent, cap) in self.coarse_caps.iter().enumerate() {
                    if !cap.intersects(center, radius) {
                        continue;
                    }
                    for child in parent * 16..(parent + 1) * 16 {
                        if !fine || self.fine_caps[child].intersects(center, radius) {
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

    /// Count the region's unfiltered membership without expanding the sorted cell ranges.
    pub(crate) fn count_region_stars(&self, region: &SelectedRegion, total: usize) -> (usize, usize, usize) {
        let always = total - self.offsets[CELL_COUNT];
        if region.brute_force {
            return (CELL_COUNT, total, always);
        }
        let count = region
            .cells
            .iter()
            .map(|&cell| self.offsets[cell + 1] - self.offsets[cell])
            .sum::<usize>();
        (region.cells.len(), count + always, always)
    }

    /// Use interval-wide magnitude bounds, including the always-checked tail. Outside the supported interval,
    /// every star is returned; only the later current-magnitude filter may reject it.
    pub(crate) fn select_brightness(
        &self,
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
                append_bright(keys, self.offsets[cell]..self.offsets[cell + 1], threshold, indices);
            }
            append_bright(keys, self.offsets[CELL_COUNT]..stars.len(), threshold, indices);
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
