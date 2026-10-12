//! The geometry of a sky tile: a gnomonic (flat tangent-plane) image of fixed size around a region's centre,
//! sky-fixed, covering the region's bounding cap plus the star drift margin plus the glyph radius. The size is the
//! same for every region (`TILE_EDGE_TEXELS` per edge, owner's decision of 2026-10-12), so the texels per radian
//! follow from the region's cap and never from the view. Texel coordinates follow the screen's convention: texel
//! `i` is centred at pixel coordinate `i`, columns grow the way screen columns grow to the right, rows grow
//! downwards from celestial north.
use crate::astro::Vector3;
use crate::constants::{CELL_COUNT, STELLAR_DRIFT_MARGIN, TILE_EDGE_TEXELS};
use crate::model::region_caps;
use crate::regions::appearance::STAR_GLYPH_RADIUS;
use std::sync::OnceLock;

/// Where a region's tile lies on the sky and how it is sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGeometry {
    center: Vector3,                        // unit, catalog axes: the region centre, at the tile's middle
    east: Vector3,                          // tangent axis of growing columns: as seen from inside the sphere with north up, screen right
    north: Vector3,                         // tangent axis of shrinking rows: celestial north at the centre
    density: f64,                           // texels per radian at the centre (per tangent-plane unit)
}

impl TileGeometry {
    /// The tile of `region`: `TILE_EDGE_TEXELS` texels wide, scaled so that the region's bounding cap, widened by
    /// the star drift margin (stars are filed by their catalog-epoch direction), fits inside the glyph radius.
    pub fn new(region: usize) -> Self {
        let cap = region_caps()[region];
        let center = cap.center;
        let pole = if center.z.abs() < 0.9 { Vector3 { x: 0.0, y: 0.0, z: 1.0 } } else { Vector3 { x: 1.0, y: 0.0, z: 0.0 } }; // the x axis stands in near the celestial poles
        let north = (pole - center * pole.dot(center)).normalized();
        let east = center.cross(north);                                                    // right = forward × up, as the camera does it
        let reach = Self::middle() - STAR_GLYPH_RADIUS as f64;                             // texels from the middle to where a glyph may still fit
        let density = reach / (cap.radius + STELLAR_DRIFT_MARGIN).tan();
        Self { center, east, north, density }
    }

    pub fn center(&self) -> Vector3 { self.center }
    pub fn density(&self) -> f64 { self.density }
    pub const fn size() -> usize { TILE_EDGE_TEXELS }
    pub const fn texel_count() -> usize { TILE_EDGE_TEXELS * TILE_EDGE_TEXELS }
    /// The pixel coordinate of the region centre along either axis: between the two middle texels.
    pub fn middle() -> f64 { (TILE_EDGE_TEXELS as f64 - 1.0) / 2.0 }

    /// Tile pixel coordinates (column, row; texel centres at integers) of a catalog direction of any length, or
    /// `None` for a direction at or behind the tangent plane.
    pub fn project(&self, direction: Vector3) -> Option<(f64, f64)> {
        let depth = direction.dot(self.center);
        if depth <= 1e-9 { return None; }
        let scale = self.density / depth;
        let middle = Self::middle();
        Some((middle + direction.dot(self.east) * scale, middle - direction.dot(self.north) * scale))
    }

    /// The unit catalog direction at tile pixel coordinates (column, row): the inverse of `project`.
    pub fn direction_of(&self, column: f64, row: f64) -> Vector3 {
        let middle = Self::middle();
        (self.center + self.east * ((column - middle) / self.density) + self.north * ((middle - row) / self.density)).normalized()
    }
}

/// A tile's geometry in f32 for the per-pixel warp: the same numbers as `TileGeometry`, packed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileFrame {
    pub center: [f32; 3],
    pub east: [f32; 3],
    pub north: [f32; 3],
    pub density: f32,                       // texels per radian
}

impl From<TileGeometry> for TileFrame {
    fn from(tile: TileGeometry) -> Self {
        let pack = |v: Vector3| [v.x as f32, v.y as f32, v.z as f32];
        Self { center: pack(tile.center), east: pack(tile.east), north: pack(tile.north), density: tile.density as f32 }
    }
}

/// The tile geometry of every region, built once (fixed geometry) for the warp's per-pixel lookups.
pub fn tile_frames() -> &'static [TileFrame] {
    static FRAMES: OnceLock<Vec<TileFrame>> = OnceLock::new();
    FRAMES.get_or_init(|| (0..CELL_COUNT).map(|region| TileGeometry::new(region).into()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::GRID_DEPTH;
    use crate::model::hash_direction;

    #[test]
    fn the_region_centre_is_the_middle_of_the_tile_and_projection_inverts() {
        for direction in [Vector3 { x: 1.0, y: 0.2, z: -0.1 }, Vector3 { x: 0.1, y: 0.1, z: 1.0 }, Vector3 { x: -0.3, y: 0.9, z: 0.3 }] {
            let region = hash_direction(GRID_DEPTH, direction);
            let tile = TileGeometry::new(region);
            let (column, row) = tile.project(tile.center()).unwrap();
            assert!((column - TileGeometry::middle()).abs() < 1e-9 && (row - TileGeometry::middle()).abs() < 1e-9);
            assert_eq!((TileGeometry::size(), TileGeometry::texel_count(), TileGeometry::middle()), (64, 4096, 31.5));
            for (c, r) in [(0.0, 0.0), (3.5, 10.25), (63.0, 2.0)] {
                let back = tile.project(tile.direction_of(c, r)).unwrap();
                assert!((back.0 - c).abs() < 1e-7 && (back.1 - r).abs() < 1e-7, "{back:?} != ({c}, {r})");
            }
            assert_eq!(tile.project(tile.center() * -1.0), None);
            assert_eq!(tile.project(tile.east), None);                                      // on the tangent plane's horizon
        }
    }

    #[test]
    fn north_is_up_and_east_is_left_as_seen_from_inside() {
        let region = hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.0, z: 0.0 });            // near RA 0, Dec 0
        let tile = TileGeometry::new(region);
        let center = tile.center();
        let north = (center + Vector3 { x: 0.0, y: 0.0, z: 0.01 }).normalized();
        let east = (center + Vector3 { x: 0.0, y: 0.01, z: 0.0 }).normalized();            // growing right ascension
        let (_, row_north) = tile.project(north).unwrap();
        let (column_east, _) = tile.project(east).unwrap();
        assert!(row_north < TileGeometry::middle() - 5.0);                                   // north is up
        assert!(column_east < TileGeometry::middle() - 5.0);                                 // east is to the left on the sky
    }

    #[test]
    fn the_tile_covers_the_region_cap_the_drift_margin_and_the_glyph_at_every_region() {
        for direction in [Vector3 { x: 0.6, y: 0.5, z: 0.4 }, Vector3 { x: 1.0, y: 0.99, z: 0.99 }, Vector3 { x: 0.0, y: 0.0, z: 1.0 }] {
            let region = hash_direction(GRID_DEPTH, direction);
            let tile = TileGeometry::new(region);
            let cap = region_caps()[region];
            let limit = (cap.radius + STELLAR_DRIFT_MARGIN).tan();
            for axis in [tile.east, tile.north, tile.east * -1.0, tile.north * -1.0] {
                let edge = (cap.center + axis * limit).normalized();                       // a star at the covered angular limit
                let (column, row) = tile.project(edge).unwrap();
                let reach = STAR_GLYPH_RADIUS as f64;
                assert!(column >= reach - 1e-6 && column <= 63.0 - reach + 1e-6 && row >= reach - 1e-6 && row <= 63.0 - reach + 1e-6, "{column}, {row}");
            }
            assert!(tile.density() > 0.0);
        }
        let corner = TileGeometry::new(hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.99, z: 0.99 }));
        let face = TileGeometry::new(hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.0, z: 0.0 }));
        assert!(corner.density() > face.density());                                          // cube corners hold smaller regions: finer texels there
    }

    #[test]
    fn the_packed_frames_match_the_geometry_of_every_region() {
        let frames = tile_frames();
        assert_eq!(frames.len(), CELL_COUNT);
        for region in [0, 1, 777, CELL_COUNT / 2, CELL_COUNT - 1] {
            assert_eq!(frames[region], TileFrame::from(TileGeometry::new(region)));
            assert!((frames[region].density - TileGeometry::new(region).density() as f32).abs() < 1e-3);
        }
    }
}
