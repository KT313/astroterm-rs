//! The same independent region loop runs on the caller or on an assigned worker. Inputs contain only
//! borrowed merged tiles; star tables, simulation caches and other regions are not accessible to a task.
use crate::constants::TILE_EDGE_TEXELS;
use crate::formulas::Vector3;
use crate::model::{Region, RegionTileCache, SkyCell, Texel};
use crate::observation::Observation;
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::time::StepTimes;
use super::pixel_warp::{RegionMapping, Screen, draw_region};
use super::tile_sampling::TileSampler;
use super::options::ProjectionOptions;
use super::worker_image::TouchedBlocks;

/// One tile to draw, a region's or a cell's it was split into, with its read-only image. Nothing is copied from it.
pub(super) struct SelectedTile<'a> {
    cell: SkyCell,
    image: &'a RegionTileCache,
}

/// The tiles of the selection: each selected region's merged tile, or the child tiles step 4 drew for the cells
/// the selection split it into.
pub(super) fn select_region_tiles<'a>(regions: &'a [Region], selected: &Selected) -> Vec<SelectedTile<'a>> {
    // Preserve selection order while restricting access to the images needed for projection.
    let mut tiles = Vec::with_capacity(selected.regions.len() + selected.leaves.len());
    for (region_index, leaves) in selected.regions_with_leaves() {
        let region = &regions[region_index];
        if leaves.is_empty() {
            tiles.push(SelectedTile { cell: SkyCell::region(region_index), image: &region.merged_tile });
        } else {
            tiles.extend(region.child_tiles.iter().map(|child| SelectedTile { cell: child.cell, image: &child.image }));
        }
    }
    tiles
}

/// Rough stereographic pixel-area weights for scheduling, not visibility or image generation.
/// Cached angular radii and one centre-direction dot product replace mapping construction and pixel scans.
/// Refraction, aberration, tile padding and sparse contents are deliberately not modelled precisely.
pub(super) fn estimate_stereographic_work(work: &mut Vec<f64>, tiles: &[SelectedTile<'_>], view_direction: &Vector3, camera_scale: f64, size: usize, fov_degrees: f64) {
    // Camera scale and view-edge alignment are common to all regions and calculated only once.
    let diameter_scale = (size as f64 - 1.0) * camera_scale;
    let view_edge_depth = (fov_degrees.to_radians() * 0.5).cos();
    let image_area = (size as f64) * (size as f64);
    work.clear();
    work.reserve(tiles.len());
    for tile in tiles {
        // Keep a small baseline cost for visiting empty images; never remove a region from drawing.
        if tile.image.is_empty() || tile.image.content_radius == Some(0.0) {
            work.push(1.0);
            continue;
        }

        // Give likely off-screen centres only the baseline cost, allowing for radius at the view edge.
        let cap = tile.cell.cap();
        let depth = cap.center.dot(*view_direction).clamp(-1.0, 1.0);
        if depth < view_edge_depth - cap.radius {
            work.push(1.0);
            continue;
        }

        // Stereographic magnification grows as 1/(1 + depth). Square the estimated width for area,
        // capped at the whole image so the opposite viewing direction cannot produce infinite work.
        let area = if depth <= -1.0 {
            image_area
        } else {
            let width = diameter_scale * cap.radius / (1.0 + depth);
            (width * width).min(image_area)
        };
        work.push(area.max(1.0));
    }
}

/// Draw each assigned region completely before moving to the next. The caller supplies a clear output
/// image; this loop never clears it or reads another task's inputs. Every task reuses one sampler.
/// Workers record written screen blocks; direct execution compiles out that bookkeeping.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_regions<const TRACK_BLOCKS: bool>(stars: &mut [Texel], touched: &mut TouchedBlocks, tiles: &[SelectedTile<'_>], screen: &Screen, observation: &Observation, options: &ProjectionOptions, times: &mut StepTimes) {
    // Reuse filter scratch across this task's regions instead of storing a sampler for each region.
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS, options);
    for tile in tiles {
        // Skip images that cannot contribute any light to the result.
        let image = tile.image;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }

        // Calculate this tile's mapping using only its cell and the common camera/observer inputs.
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::of_cell(tile.cell);
        times.measure("Projection.Region mapping");
        let Some(mapping) = RegionMapping::new(&geometry, observation, screen, image.content_radius) else { continue; };

        // Draw directly into the task's output, retaining region order wherever images overlap.
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows(), image.filter_summary.as_ref());
        times.measure("Projection.Tile rasterization");
        draw_region::<TRACK_BLOCKS>(stars, touched, &mapping, &sampler, image.texels());
    }
}
