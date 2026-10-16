//! Frame-local screen corrections shared by overlapping region images. Horizontal strips bound scratch use;
//! each strip visits regions in their original order, so source-over composition is unchanged.
use crate::constants::{STEREOGRAPHIC_ROW_REBASE_PIXELS, TILE_EDGE_TEXELS};
use crate::formulas::{LIGHT_SPEED_AU_DAY, Matrix3, Vector3, unrefract_direction};
use crate::model::{Region, RegionTileCache, Texel};
use crate::observation::Observation;
use crate::regions::TileGeometry;
use crate::time::StepTimes;
use super::pixel_warp::{RegionMapping, RowRays, Screen, draw_region, sample_and_blend_pixel};
use super::tile_sampling::TileSampler;

/// One region's drawing inputs for this frame. Masks and mapping are prepared once, not once per strip.
struct PreparedRegion<'a> {
    mapping: RegionMapping<'a>,
    sampler: TileSampler,
    image: &'a RegionTileCache,
    first_row: usize,
    last_row: usize,
}

/// Corrections constant over a frame, independent of the region that will sample a pixel.
struct ScreenCorrections {
    to_catalog: Matrix3,
    aberration: Vector3,
    atmosphere: bool,
}

impl ScreenCorrections {
    fn new(observation: &Observation) -> Self {
        let aberration = observation.to_horizon.apply(observation.observer.velocity * (1.0 / LIGHT_SPEED_AU_DAY));
        Self { to_catalog: observation.to_catalog, aberration, atmosphere: observation.atmosphere }
    }

    #[inline]
    fn calculate(&self, numerator: Vector3, spread: f64, factor: f64) -> CorrectedRay {
        // Undo the atmospheric bend, when enabled, before subtracting the observer's velocity over c.
        let (direction, length) = if self.atmosphere {
            let direction = unrefract_direction(numerator * (1.0 / spread)) - self.aberration;
            (direction, direction.length())
        } else {
            // Keep the existing homogeneous direction and first-order length approximation.
            (numerator - self.aberration * spread, spread - numerator.dot(self.aberration))
        };

        // Regions need only their own tile-basis rotation after this common catalog rotation.
        CorrectedRay { direction: self.to_catalog.apply(direction), length, factor }
    }
}

#[derive(Clone, Copy, Default)]
struct CorrectedRay {
    direction: Vector3,
    length: f64,
    factor: f64,
}

/// Scratch for one strip, overwritten by prepare before use. Its effective key is the current screen and
/// observation plus first_row..end_row; it is dropped at frame end and never reused as a cached frame.
struct SharedScreenRows {
    rays: Vec<CorrectedRay>,
    width: usize,
    first_row: usize,
    end_row: usize,
    #[cfg(test)]
    prepared_pixels: usize,
}

impl SharedScreenRows {
    fn new(width: usize, rows: usize) -> Self {
        Self {
            rays: vec![CorrectedRay::default(); width.checked_mul(rows).expect("shared screen size overflow")],
            width,
            first_row: 0,
            end_row: 0,
            #[cfg(test)]
            prepared_pixels: 0,
        }
    }

    fn prepare(&mut self, screen: &Screen, corrections: &ScreenCorrections, first_row: usize, end_row: usize) {
        // Only this row range is valid after preparation; pixels outside the disc are never sampled.
        self.first_row = first_row;
        self.end_row = end_row;
        for row in first_row..end_row {
            let Some((first_column, last_column)) = screen.disc_columns(row) else { continue; };
            let start = (row - first_row) * self.width + first_column;
            let rays = &mut self.rays[start..=start + last_column - first_column];

            // Restart long screen rows periodically, just as the transformed stereographic path does.
            for (chunk, rays) in rays.chunks_mut(STEREOGRAPHIC_ROW_REBASE_PIXELS).enumerate() {
                let column = first_column + chunk * STEREOGRAPHIC_ROW_REBASE_PIXELS;
                let mut screen_rays = RowRays::new(screen, column as f64, row as f64);
                for ray in rays {
                    let (numerator, spread, factor) = screen_rays.current(screen);
                    screen_rays.advance();
                    *ray = corrections.calculate(numerator, spread, factor);
                    #[cfg(test)]
                    { self.prepared_pixels += 1; }
                }
            }
        }
    }

    fn span(&self, row: usize, first_column: usize, last_column: usize) -> &[CorrectedRay] {
        debug_assert!((self.first_row..self.end_row).contains(&row));
        let start = (row - self.first_row) * self.width + first_column;
        &self.rays[start..=start + last_column - first_column]
    }
}

pub(super) fn project_shared_tiles(stars: &mut [Texel], screen: &Screen, regions: &[Region], selected: &[usize], observation: &Observation, times: &mut StepTimes, strip_height: usize) {
    // Prepare each region once, keeping selection order even when some images are empty or off-screen.
    assert!(strip_height > 0, "shared screen strips must contain at least one row");
    let prepared = prepare_regions(screen, regions, selected, observation, times);
    if prepared.is_empty() { return; }
    times.measure("Projection.Screen strip indexing");
    let strips = index_regions_by_strip(&prepared, screen.size, strip_height);

    // Avoid computing a whole strip of corrections when the candidate pixels do not offer any reuse.
    if !has_repeated_screen_work(&prepared, &strips, screen, strip_height) {
        times.measure("Projection.Tile rasterization");
        for region in &prepared { draw_region(stars, &region.mapping, &region.sampler, region.image.texels()); }
        return;
    }

    draw_shared_strips(stars, screen, &prepared, &strips, observation, times, strip_height);
}

/// Draw disjoint strips with shared corrections, preserving region order within every destination pixel.
fn draw_shared_strips(stars: &mut [Texel], screen: &Screen, prepared: &[PreparedRegion<'_>], strips: &[Vec<usize>], observation: &Observation, times: &mut StepTimes, strip_height: usize) {
    // Reuse bounded scratch across disjoint strips, calculating each needed screen row at most once.
    times.measure("Projection.Shared screen preparation");
    let corrections = ScreenCorrections::new(observation);
    let mut shared = SharedScreenRows::new(screen.size, strip_height.min(screen.size));
    for (strip, region_indices) in strips.iter().enumerate() {
        if region_indices.is_empty() { continue; }
        let first_row = strip * strip_height;
        let end_row = (first_row + strip_height).min(screen.size);
        times.measure("Projection.Shared screen preparation");
        shared.prepare(screen, &corrections, first_row, end_row);

        // Each pixel sees the same region order as before; different strips contain disjoint pixels.
        times.measure("Projection.Tile rasterization");
        for &index in region_indices { draw_shared_region(stars, &prepared[index], &shared); }
    }
}

fn prepare_regions<'a>(screen: &'a Screen, regions: &'a [Region], selected: &[usize], observation: &Observation, times: &mut StepTimes) -> Vec<PreparedRegion<'a>> {
    let mut prepared = Vec::with_capacity(selected.len());
    for &index in selected {
        let image = &regions[index].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }

        // Keep the same image support and mapping equations used by the unshared path.
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(index);
        times.measure("Projection.Region mapping");
        let Some(mapping) = RegionMapping::new(&geometry, observation, screen, image.content_radius) else { continue; };
        let (first_row, last_row) = mapping.rows();
        if first_row > last_row { continue; }

        // Store the masks once so visiting another strip never rebuilds a tile's sampling data.
        times.measure("Projection.Tile filter preparation");
        let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
        sampler.prepare(image.lit_rows());
        prepared.push(PreparedRegion { mapping, sampler, image, first_row, last_row });
    }
    prepared
}

fn index_regions_by_strip(regions: &[PreparedRegion<'_>], size: usize, strip_height: usize) -> Vec<Vec<usize>> {
    let mut strips = vec![Vec::new(); size.div_ceil(strip_height)];
    for (index, region) in regions.iter().enumerate() {
        let first_strip = region.first_row / strip_height;
        let last_strip = region.last_row / strip_height;
        for strip in &mut strips[first_strip..=last_strip] { strip.push(index); }
    }
    strips
}

fn has_repeated_screen_work(regions: &[PreparedRegion<'_>], strips: &[Vec<usize>], screen: &Screen, strip_height: usize) -> bool {
    // Compare geometry visits with the number of screen pixels that strip preparation would evaluate.
    let mut candidate_pixels = 0_usize;
    for region in regions {
        for row in region.first_row..=region.last_row {
            for (first, last) in region.mapping.columns_of(row).into_iter().flatten() {
                candidate_pixels += last - first + 1;
            }
        }
    }
    let mut shared_pixels = 0_usize;
    for (strip, regions) in strips.iter().enumerate() {
        if regions.is_empty() { continue; }
        for row in strip * strip_height..((strip + 1) * strip_height).min(screen.size) {
            if let Some((first, last)) = screen.disc_columns(row) { shared_pixels += last - first + 1; }
        }
    }
    candidate_pixels > shared_pixels
}

fn draw_shared_region(stars: &mut [Texel], region: &PreparedRegion<'_>, shared: &SharedScreenRows) {
    // Clip the region's rows to this strip, retaining the original sparse row and column rejection.
    let first_row = region.first_row.max(shared.first_row);
    let last_row = region.last_row.min(shared.end_row - 1);
    for row in first_row..=last_row {
        for (first, last) in region.mapping.columns_of(row).into_iter().flatten() {
            let Some((first_tile_row, last_tile_row)) = region.mapping.tile_rows_of(row, first, last) else { continue; };
            if !region.sampler.any_light_in_rows(first_tile_row, last_tile_row) { continue; }
            let pixels = &mut stars[row * shared.width + first..=row * shared.width + last];
            let rays = shared.span(row, first, last);

            // Only the region's tile projection is left; filter and blend exactly as in the other paths.
            for (pixel, ray) in pixels.iter_mut().zip(rays) {
                if let Some((position, footprint)) = region.mapping.project_corrected_direction(ray.direction, ray.length, ray.factor) {
                    sample_and_blend_pixel(pixel, &region.sampler, region.image.texels(), position, footprint);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "shared_screen_tests.rs"]
mod tests;
