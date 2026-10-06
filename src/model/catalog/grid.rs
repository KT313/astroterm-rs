//! Cube-map storage, representation construction and shared selection records.
use crate::astro::Vector3;
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
    pub(crate) cells: Vec<usize>,
    pub(crate) brute_force: bool,
}

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

impl CellCap {
    pub(crate) fn intersects(self, center: Vector3, radius: f64) -> bool {
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
pub(crate) fn interleave(x: usize, y: usize, depth: u8) -> usize {
    (0..depth)
        .map(|bit| ((x >> bit) & 1) << (2 * bit) | ((y >> bit) & 1) << (2 * bit + 1))
        .sum()
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
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(SkyGrid { offsets, coarse_caps, fine_caps });

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(SelectedRegion { cells });
