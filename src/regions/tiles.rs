//! The geometry of a sky tile: a stereographic image of fixed size around a region's centre, sky-fixed, covering
//! the region's bounding cap plus the star drift margin plus the glyph radius. The size is the same for every
//! region (`TILE_EDGE_TEXELS` per edge, owner's decision of 2026-10-12), so the texels per radian follow from the
//! region's cap and never from the view. Stereographic rather than gnomonic (owner's decision of 2026-10-15)
//! because the screen is stereographic too: a rotation of the sky then maps tile to screen by a closed formula,
//! which is what lets step 5 pull every screen pixel back to its texel. Texel coordinates follow the screen's
//! convention: texel `i` is centred at pixel coordinate `i`, columns grow the way screen columns grow to the
//! right, rows grow downwards from celestial north.
use crate::formulas::Vector3;
use crate::constants::{GRID_DEPTH, STELLAR_DRIFT_MARGIN, TILE_EDGE_TEXELS};
use crate::model::SkyCell;
#[cfg(test)]
use crate::model::region_caps;
use crate::regions::appearance::STAR_GLYPH_RADIUS;

/// Where a region's tile lies on the sky and how it is sampled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGeometry {
    center: Vector3,                        // unit, catalog axes: the region centre, at the tile's middle
    east: Vector3,                          // tangent axis of growing columns: as seen from inside the sphere with north up, screen right
    north: Vector3,                         // tangent axis of shrinking rows: celestial north at the centre
    density: f64,                           // texels per radian at the centre: 2·density·tan(θ/2) texels at angle θ
}

impl TileGeometry {
    /// The tile of `region`: `TILE_EDGE_TEXELS` texels wide, scaled so that the region's bounding cap, widened by
    /// the star drift margin (stars are filed by their catalog-epoch direction), fits inside the glyph radius.
    pub fn new(region: usize) -> Self { Self::of_cell(SkyCell::region(region)) }

    /// The tile of any grid cell: as `new` for a region; a deeper cell, which a zoomed-in region splits into, has no
    /// drift margin, because its stars are given to it by their current direction.
    pub fn of_cell(cell: SkyCell) -> Self {
        let center = cell.cap().center;
        let pole = if center.z.abs() < 0.9 { Vector3 { x: 0.0, y: 0.0, z: 1.0 } } else { Vector3 { x: 1.0, y: 0.0, z: 0.0 } }; // the x axis stands in near the celestial poles
        let north = (pole - center * pole.dot(center)).normalized();
        let east = center.cross(north);                                                    // right = forward × up, as the camera does it
        Self { center, east, north, density: Self::density_of(cell) }
    }

    /// The texels per radian at the centre of `cell`'s tile, without building its axes: `density()` of `of_cell`.
    pub fn density_of(cell: SkyCell) -> f64 {
        let margin = if cell.depth == GRID_DEPTH { STELLAR_DRIFT_MARGIN } else { 0.0 };
        let reach = Self::middle() - STAR_GLYPH_RADIUS as f64;                             // texels from the middle to where a glyph may still fit
        reach / (2.0 * ((cell.cap().radius + margin) / 2.0).tan())
    }

    pub fn center(&self) -> Vector3 { self.center }
    pub fn east(&self) -> Vector3 { self.east }
    pub fn north(&self) -> Vector3 { self.north }
    pub fn density(&self) -> f64 { self.density }
    pub const fn size() -> usize { TILE_EDGE_TEXELS }
    pub const fn texel_count() -> usize { TILE_EDGE_TEXELS * TILE_EDGE_TEXELS }
    /// The pixel coordinate of the region centre along either axis: between the two middle texels.
    pub fn middle() -> f64 { (TILE_EDGE_TEXELS as f64 - 1.0) / 2.0 }

    /// Tile pixel coordinates (column, row; texel centres at integers) of a catalog direction of any length, or
    /// `None` at the point opposite the centre, which the stereographic projection sends to infinity.
    pub fn project(&self, direction: Vector3) -> Option<(f64, f64)> {
        let length = direction.length();
        let depth = direction.dot(self.center);
        if length + depth <= 1e-9 * length { return None; }
        let scale = 2.0 * self.density / (length + depth);
        let middle = Self::middle();
        Some((middle + direction.dot(self.east) * scale, middle - direction.dot(self.north) * scale))
    }

    /// The unit catalog direction at tile pixel coordinates (column, row): the inverse of `project`.
    pub fn direction_of(&self, column: f64, row: f64) -> Vector3 {
        let middle = Self::middle();
        let (x, y) = ((column - middle) / self.density, (middle - row) / self.density);
        let q = (x * x + y * y) / 4.0;                                                      // tan²(θ/2) at this texel
        (self.center * (1.0 - q) + self.east * x + self.north * y) * (1.0 / (1.0 + q))     // cos θ along the centre, sin θ across: unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let (column, row) = tile.project(tile.east).unwrap();                           // 90° from the centre: 2·density texels out
            assert!((column - TileGeometry::middle() - 2.0 * tile.density()).abs() < 1e-9 && (row - TileGeometry::middle()).abs() < 1e-9);
            for theta in [0.01_f64, 0.3, 1.5, 2.5] {                                          // stereographic: 2·density·tan(θ/2) texels at angle θ
                let (column, row) = tile.project(tile.center() * theta.cos() + tile.north * theta.sin()).unwrap();
                assert!((column - TileGeometry::middle()).abs() < 1e-9, "{theta}");
                assert!((TileGeometry::middle() - row - 2.0 * tile.density() * (theta / 2.0).tan()).abs() < 1e-9, "{theta}");
            }
            assert!((tile.direction_of(0.0, 0.0).length() - 1.0).abs() < 1e-12);
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
            let (sine, cosine) = (cap.radius + STELLAR_DRIFT_MARGIN).sin_cos();
            for axis in [tile.east, tile.north, tile.east * -1.0, tile.north * -1.0] {
                let edge = cap.center * cosine + axis * sine;                               // a star at the covered angular limit
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
    fn a_child_tile_covers_its_cell_and_the_glyph_without_the_drift_margin() {
        let region = SkyCell::region(hash_direction(GRID_DEPTH, Vector3 { x: 0.6, y: 0.5, z: 0.4 }));
        assert_eq!(TileGeometry::of_cell(region), TileGeometry::new(region.index));
        for depth in [GRID_DEPTH + 1, GRID_DEPTH + 4] {
            let cell = region.locate_within(region.cap().center, depth);
            let tile = TileGeometry::of_cell(cell);
            let cap = cell.cap();
            let (sine, cosine) = cap.radius.sin_cos();
            let reach = STAR_GLYPH_RADIUS as f64;
            for axis in [tile.east, tile.north, tile.east * -1.0, tile.north * -1.0] {
                let (column, row) = tile.project(cap.center * cosine + axis * sine).unwrap();
                assert!((column - reach).abs() < 1e-6 || (column - 63.0 + reach).abs() < 1e-6 || (row - reach).abs() < 1e-6 || (row - 63.0 + reach).abs() < 1e-6, "{column}, {row}");
            }
            assert_eq!(tile.density(), TileGeometry::density_of(cell));
        }
        let child = TileGeometry::of_cell(region.children()[0]);
        assert!(child.density() > 2.0 * TileGeometry::new(region.index).density());       // half the cell, and no margin
    }

}
