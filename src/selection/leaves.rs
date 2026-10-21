//! Splitting zoomed-in regions: a region whose tile would have too few texels per screen pixel is drawn as the
//! tiles of smaller grid cells instead, each the same size as a region tile. Cells are quartered recursively until
//! their tiles are fine enough on screen, keeping only the quarters that reach into the view.
use crate::constants::{GRID_DEPTH, MAX_TILE_SPLITS, MIN_TILE_TEXELS_PER_PIXEL};
use crate::model::SkyCell;
use crate::observation::SkyPatch;
use super::tile_scale::{TileScale, estimate_texels_per_pixel};

/// Whether a tile with `texels_per_pixel` at `cell` is drawn as it is: fine enough on screen, or at the depth limit.
pub(super) fn is_fine_enough(cell: SkyCell, texels_per_pixel: f64) -> bool {
    texels_per_pixel > MIN_TILE_TEXELS_PER_PIXEL || cell.depth >= GRID_DEPTH + MAX_TILE_SPLITS
}

/// Append the cells whose tiles replace `cell`'s: its quarters that reach into the patch, each kept when its tile
/// is fine enough and quartered again otherwise. Appends nothing when no quarter reaches into the patch.
pub(super) fn split_into_leaves(cell: SkyCell, scale: &TileScale, patch: &SkyPatch, leaves: &mut Vec<SkyCell>) {
    // Only quarters in view are estimated and split further.
    let quarters: Vec<SkyCell> = cell.children().into_iter().filter(|quarter| quarter.cap().intersects(patch.center, patch.radius)).collect();
    for (quarter, texels_per_pixel) in estimate_texels_per_pixel(&quarters, scale) {
        if is_fine_enough(quarter, texels_per_pixel) { leaves.push(quarter); } else { split_into_leaves(quarter, scale, patch, leaves); }
    }
}
