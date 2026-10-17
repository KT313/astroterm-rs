//! Cube-map storage, representation construction and shared selection records.
use crate::constants::GRID_DEPTH;
use crate::rows::row_columns;
use crate::formulas::Vector3;
use std::f64::consts::PI;
use std::sync::OnceLock;

const NUMERIC_SLACK: f64 = 1e-10;

#[derive(Clone, Debug, PartialEq)]
pub struct SkyGrid {
    pub offsets: crate::catalog::cache::CatalogArray<usize>,
    pub(crate) coarse_caps: Vec<CellCap>,
    pub(crate) fine_caps: Vec<CellCap>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellCap {
    pub(crate) center: Vector3,
    pub(crate) radius: f64,
}
row_columns!(CellCap { center, radius });

impl CellCap {
    pub(crate) fn intersects(self, center: Vector3, radius: f64) -> bool {
        let sum = radius + self.radius + NUMERIC_SLACK;
        sum >= PI || self.center.dot(center) >= sum.cos()
    }
}

/// A cell of the cube-map grid at any depth. The cells at `GRID_DEPTH` are the regions; every cell splits into
/// the four cells `4·index..4·index + 4` one level deeper, because the interleaved index gains two low bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkyCell {
    pub depth: u8,
    pub index: usize,                       // face · 4^depth + the interleaved (u, v) cell position, as `hash_direction`
}

impl SkyCell {
    /// The cell of region `region`.
    pub fn region(region: usize) -> Self { Self { depth: GRID_DEPTH, index: region } }
    /// The region this cell lies in; the cell must be at `GRID_DEPTH` or deeper.
    pub fn region_index(self) -> usize { self.index >> (2 * (self.depth - GRID_DEPTH)) }
    /// The four quarters of the cell, one level deeper.
    pub fn children(self) -> [SkyCell; 4] { std::array::from_fn(|quarter| SkyCell { depth: self.depth + 1, index: 4 * self.index + quarter }) }

    /// The bounding cap; a region's comes from the shared table, deeper cells' are calculated.
    pub(crate) fn cap(self) -> CellCap {
        if self.depth == GRID_DEPTH { return region_caps()[self.index]; }
        let (face, x, y) = self.locate_on_face();
        find_cell_cap(face, x, y, 1 << self.depth)
    }

    /// The cell `depth` levels deep inside this one that holds `direction`, or the nearest such cell when
    /// `direction` lies outside this cell: its face coordinates are clamped into this cell's square.
    pub fn locate_within(self, direction: Vector3, depth: u8) -> SkyCell {
        let (face, x, y) = self.locate_on_face();
        let (u, v) = project_onto_face(face, direction);
        let (shift, n) = (depth - self.depth, 1_usize << depth);
        let index = |q: f64, low: usize| (((q + 1.0) * 0.5 * n as f64).floor().max(0.0) as usize).clamp(low << shift, ((low + 1) << shift) - 1);
        SkyCell { depth, index: face * n * n + interleave(index(u, x), index(v, y), depth) }
    }

    /// The face and the column and row of the cell in that face's grid.
    fn locate_on_face(self) -> (usize, usize, usize) {
        let position = self.index & ((1 << (2 * self.depth)) - 1);
        let (x, y) = deinterleave(position, self.depth);
        (self.index >> (2 * self.depth), x, y)
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

/// The (u, v) coordinates of `p` on the plane of `face`, as `hash_direction` computes them for the face it picks.
fn project_onto_face(face: usize, p: Vector3) -> (f64, f64) {
    match face {
        0 | 1 => (p.y / p.x.abs(), p.z / p.x.abs()),
        2 | 3 => (p.x / p.y.abs(), p.z / p.y.abs()),
        _ => (p.x / p.z.abs(), p.y / p.z.abs()),
    }
}

pub(crate) fn interleave(x: usize, y: usize, depth: u8) -> usize {
    (0..depth)
        .map(|bit| ((x >> bit) & 1) << (2 * bit) | ((y >> bit) & 1) << (2 * bit + 1))
        .sum()
}

/// The column and row that `interleave` combined.
fn deinterleave(code: usize, depth: u8) -> (usize, usize) {
    (0..depth).fold((0, 0), |(x, y), bit| (x | ((code >> (2 * bit)) & 1) << bit, y | ((code >> (2 * bit + 1)) & 1) << bit))
}
pub(crate) fn direction(face: usize, u: f64, v: f64) -> Vector3 {
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
pub(crate) fn build_caps(depth: u8) -> Vec<CellCap> {
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
                caps[face * n * n + interleave(x, y, depth)] = find_cell_cap(face, x, y, n);
            }
        }
    }
    caps
}

/// The bounding cap of cell (`x`, `y`) of `face` in an `n` × `n` face grid: around the cell's middle, reaching its
/// farthest corner.
fn find_cell_cap(face: usize, x: usize, y: usize, n: usize) -> CellCap {
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
    CellCap { center, radius }
}

/// The bounding caps of the `GRID_DEPTH` regions: fixed geometry, built on first use and shared by the selection
/// and the tiles.
pub(crate) fn region_caps() -> &'static [CellCap] {
    static CAPS: OnceLock<Vec<CellCap>> = OnceLock::new();
    CAPS.get_or_init(|| build_caps(GRID_DEPTH))
}

impl SkyGrid {
    pub(crate) fn from_offsets(offsets: crate::catalog::cache::CatalogArray<usize>) -> Self {
        Self {
            offsets,
            coarse_caps: build_caps(crate::constants::GRID_COARSE_DEPTH),
            fine_caps: build_caps(GRID_DEPTH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directions() -> impl Iterator<Item = Vector3> {
        (0..400).map(|i| {
            let (a, b) = (i as f64 * 0.7548776662, i as f64 * 0.5698402910);              // a low-discrepancy spiral over the sphere
            let z = 2.0 * (a.fract()) - 1.0;
            let phi = std::f64::consts::TAU * b.fract();
            Vector3 { x: (1.0 - z * z).sqrt() * phi.cos(), y: (1.0 - z * z).sqrt() * phi.sin(), z }
        })
    }

    #[test]
    fn deeper_cells_are_the_children_of_the_cells_that_hold_them() {
        for direction in directions() {
            let region = SkyCell::region(hash_direction(GRID_DEPTH, direction));
            let mut cell = region;
            for depth in GRID_DEPTH + 1..GRID_DEPTH + 6 {
                let deeper = SkyCell { depth, index: hash_direction(depth, direction) };
                assert!(cell.children().contains(&deeper), "{direction:?} depth {depth}");
                assert_eq!(deeper.region_index(), region.index);
                assert_eq!(region.locate_within(direction, depth), deeper);
                cell = deeper;
            }
        }
    }

    #[test]
    fn cell_caps_hold_their_cells_and_match_the_region_table() {
        let caps = build_caps(GRID_DEPTH + 2);
        for direction in directions() {
            let region = SkyCell::region(hash_direction(GRID_DEPTH, direction));
            assert_eq!(region.cap(), region_caps()[region.index]);
            let cell = region.locate_within(direction, GRID_DEPTH + 2);
            let (face, x, y) = cell.locate_on_face();
            assert_eq!(find_cell_cap(face, x, y, 1 << cell.depth), caps[cell.index]);
            assert_eq!(cell.cap(), caps[cell.index]);
            let angle = cell.cap().center.cross(direction).length().atan2(cell.cap().center.dot(direction));
            assert!(angle <= cell.cap().radius, "{direction:?} {cell:?}: {angle} > {}", cell.cap().radius);
        }
    }

    #[test]
    fn directions_outside_a_cell_go_to_its_nearest_edge_cell() {
        let region = SkyCell::region(hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.3, z: 0.2 }));
        let cap = region.cap();
        let east = Vector3 { x: 0.0, y: 0.0, z: 1.0 }.cross(cap.center).normalized();    // growing u on face 0
        let outside = (cap.center + east * (2.0 * cap.radius)).normalized();
        let inside = (cap.center + east * (0.6 * cap.radius)).normalized();
        let depth = GRID_DEPTH + 3;
        assert_ne!(hash_direction(GRID_DEPTH, outside), region.index);
        let clamped = region.locate_within(outside, depth);
        assert_eq!(clamped.region_index(), region.index);
        let (_, x, _) = clamped.locate_on_face();
        let (_, region_x, _) = region.locate_on_face();
        assert_eq!(x, ((region_x + 1) << 3) - 1);                                           // the last column of the region
        assert_eq!(region.locate_within(inside, depth).index, hash_direction(depth, inside));
    }

    #[test]
    fn deinterleave_inverts_interleave() {
        for (x, y) in [(0, 0), (5, 9), (31, 0), (0, 31), (1023, 777)] {
            assert_eq!(deinterleave(interleave(x, y, 10), 10), (x, y));
        }
    }
}
