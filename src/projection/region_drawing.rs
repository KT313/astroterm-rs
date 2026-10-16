//! The same independent region loop runs on the caller or on an assigned worker. Inputs contain only
//! borrowed merged tiles; star tables, simulation caches and other regions are not accessible to a task.
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Region, RegionTileCache, Texel};
use crate::observation::Observation;
use crate::regions::TileGeometry;
use crate::time::StepTimes;
use super::pixel_warp::{RegionMapping, Screen, draw_region};
use super::tile_sampling::TileSampler;

/// One selected region's identity and read-only image. Nothing is copied from its cached tile.
pub(super) struct SelectedTile<'a> {
    region_index: usize,
    image: &'a RegionTileCache,
}

pub(super) fn select_region_tiles<'a>(regions: &'a [Region], selected: &[usize]) -> Vec<SelectedTile<'a>> {
    // Preserve selection order while restricting access to the images needed for projection.
    let mut tiles = Vec::with_capacity(selected.len());
    for &region_index in selected {
        tiles.push(SelectedTile { region_index, image: &regions[region_index].merged_tile });
    }
    tiles
}

/// Draw each assigned region completely before moving to the next. The caller supplies a clear output
/// image; this loop never clears it or reads another task's inputs. Every task reuses one sampler.
pub(super) fn draw_regions(stars: &mut [Texel], tiles: &[SelectedTile<'_>], screen: &Screen, observation: &Observation, times: &mut StepTimes) {
    // Reuse filter scratch across this task's regions instead of storing a sampler for each region.
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    for tile in tiles {
        // Skip images that cannot contribute any light to the result.
        let image = tile.image;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }

        // Calculate this region's mapping using only its identity and the common camera/observer inputs.
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(tile.region_index);
        times.measure("Projection.Region mapping");
        let Some(mapping) = RegionMapping::new(&geometry, observation, screen, image.content_radius) else { continue; };

        // Draw directly into the task's output, retaining region order wherever images overlap.
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows(), image.filter_summary.as_ref());
        times.measure("Projection.Tile rasterization");
        draw_region(stars, &mapping, &sampler, image.texels());
    }
}
