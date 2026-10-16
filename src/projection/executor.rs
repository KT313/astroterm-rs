//! Optional projection-only worker pool. Every task runs the same independent region loop into its own
//! image. The caller combines consecutive region chunks in selection order, after all tasks finish.
use std::io;
use rayon::{ThreadPool, ThreadPoolBuilder};
use crate::model::{ProjectionKind, Region, Texel, View};
use crate::observation::Observation;
use crate::regions::blend_star;
use crate::time::StepTimes;
use super::prepare_camera;
use super::pixel_warp::Screen;
use super::region_drawing::{SelectedTile, draw_regions, estimate_stereographic_work, select_region_tiles};

/// Lives outside the frame loop. Zero workers runs the region loop locally; positive counts use a
/// dedicated pool, never Rayon's global pool. Buffers retain capacity only, not valid image contents.
#[derive(Default)]
pub struct ProjectionExecutor {
    pool: Option<ThreadPool>,
    buffers: Vec<Vec<Texel>>,                  // extra chunk images; chunk zero draws into the returned image
}

impl ProjectionExecutor {
    /// Create exactly the requested worker count, or no pool at all for direct execution.
    pub fn new(workers: usize) -> io::Result<Self> {
        if workers == 0 { return Ok(Self::default()); }
        if workers > rayon::max_num_threads() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                format!("Requested {workers} projection workers exceeds the supported maximum {}", rayon::max_num_threads())));
        }

        // Name workers for profilers and report startup failures instead of silently running serially.
        let pool = ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|index| format!("projection-{index}"))
            .build()
            .map_err(|error| io::Error::other(format!("Cannot start {workers} projection workers: {error}")))?;
        Ok(Self { pool: Some(pool), buffers: Vec::new() })
    }

    pub(super) fn project(&mut self, size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, times: &mut StepTimes) -> Vec<Texel> {
        // Allocate the final image and prepare common, immutable camera inputs on the calling thread.
        times.measure("Projection.Star buffer allocation");
        let pixel_count = size.checked_mul(size).expect("drawing area pixel count overflow");
        let mut stars = vec![Texel::default(); pixel_count];
        if size < 2 || selected.is_empty() { return stars; }
        times.measure("Projection.Camera preparation");
        let camera = prepare_camera(view);
        let screen = Screen::new(camera, size);
        times.measure("Projection.Tile selection");
        let tiles = select_region_tiles(regions, selected);

        // Direct execution uses exactly the same loop as each worker, with the normal frame timer.
        let Some(pool) = &self.pool else {
            draw_regions(&mut stars, &tiles, &screen, observation, times);
            times.measure("Projection");
            return stars;
        };

        // Estimate only when stereographic rendering has multiple tasks. Other paths keep equal counts.
        let workers = pool.current_num_threads();
        let chunks = if workers > 1 && tiles.len() > 1 && view.projection == ProjectionKind::Stereographic {
            times.measure("Projection.Work estimation");
            let work = estimate_stereographic_work(&tiles, &observation.patch.center, camera.scale, size, view.fov_degrees);
            split_regions_by_work(&tiles, &work, workers)
        } else {
            split_region_chunks(&tiles, workers)
        };

        // Only additional chunks need temporary images; every chunk retains the original region order.
        times.measure("Projection.Worker preparation");
        self.buffers.resize_with(chunks.len() - 1, Vec::new);
        for buffer in &mut self.buffers {
            buffer.clear();
            buffer.resize(pixel_count, Texel::default());
        }

        // Submit one job per chunk for the entire frame. Each task borrows only its assigned tiles.
        times.measure("Projection.Worker rasterization");
        draw_region_chunks(pool, &chunks, &mut stars, &mut self.buffers, &screen, observation);

        // Chunk zero is already in the final image. A one-worker result needs no copying or composition.
        if !self.buffers.is_empty() {
            times.measure("Projection.Worker composition");
            combine_chunk_buffers(&mut stars, &self.buffers);
        }
        times.measure("Projection");
        stars
    }
}

/// Return contiguous borrowed slices without exposing the rest of the selected regions to a task.
fn split_region_chunks<T>(regions: &[T], workers: usize) -> Vec<&[T]> {
    assert!(workers > 0);
    let count = workers.min(regions.len());
    if count == 0 { return Vec::new(); }

    // Give the first few tasks one extra region when the division has a remainder.
    let base = regions.len() / count;
    let extra = regions.len() % count;
    let mut chunks = Vec::with_capacity(count);
    let mut remaining = regions;
    for index in 0..count {
        let length = base + usize::from(index < extra);
        let (chunk, rest) = remaining.split_at(length);
        chunks.push(chunk);
        remaining = rest;
    }
    chunks
}

/// Balance approximate work without reordering or splitting individual regions. Boundaries follow
/// cumulative work targets; reserving one region per remaining task prevents empty chunks.
fn split_regions_by_work<'a, T>(regions: &'a [T], work: &[f64], workers: usize) -> Vec<&'a [T]> {
    assert!(workers > 0);
    assert_eq!(regions.len(), work.len());
    let count = workers.min(regions.len());
    let total_work: f64 = work.iter().sum();
    if count <= 1 || total_work == 0.0 { return split_region_chunks(regions, workers); }

    // Walk the selected list once, cutting when the next share of total estimated work has been reached.
    let mut chunks = Vec::with_capacity(count);
    let mut first = 0;
    let mut completed_work = 0.0;
    for (index, &cost) in work.iter().enumerate() {
        let remaining_chunks = count - chunks.len() - 1;
        if remaining_chunks == 0 { break; }
        completed_work += cost;
        let target = total_work * ((chunks.len() + 1) as f64 / count as f64);
        let remaining_regions = regions.len() - index - 1;
        if completed_work >= target || remaining_regions == remaining_chunks {
            chunks.push(&regions[first..=index]);
            first = index + 1;
        }
    }

    // Always retain the final suffix, including cheap regions after the last expensive one.
    chunks.push(&regions[first..]);
    chunks
}

fn draw_region_chunks(pool: &ThreadPool, chunks: &[&[SelectedTile<'_>]], stars: &mut [Texel], buffers: &mut [Vec<Texel>], screen: &Screen, observation: &Observation) {
    // The first task writes the final image; every other task gets a separate full-size output.
    let outputs = std::iter::once(stars).chain(buffers.iter_mut().map(Vec::as_mut_slice));
    pool.in_place_scope(|scope| {
        for (&tiles, output) in chunks.iter().zip(outputs) {
            scope.spawn(move |_| {
                // Worker-local checkpoints never mutate the caller's timer or mix CPU time with wall time.
                let mut times = StepTimes::default();
                draw_regions(output, tiles, screen, observation, &mut times);
            });
        }
    });
}

/// Source-over is associative in real arithmetic for premultiplied tiles. Chunking changes f32 rounding;
/// it does not permit reordering chunks or applying the speed-group weighted merge to their outputs.
/// Tile creation keeps RGB <= opacity: palette colour is multiplied by opacity, source-over preserves the
/// bound, and group averaging is <= the larger input opacity (which is <= their combined opacity). The box
/// filter preserves it too. Consequently per-region clamping only absorbs roundoff for these inputs.
fn combine_chunk_buffers(destination: &mut [Texel], buffers: &[Vec<Texel>]) {
    // The destination already contains the earliest chunk; add subsequent chunks in selection order.
    for buffer in buffers {
        assert_eq!(destination.len(), buffer.len(), "projection chunk images must have matching dimensions");
        for (pixel, source) in destination.iter_mut().zip(buffer) {
            if source.is_empty() { continue; }
            blend_star(pixel, source.rgb, source.opacity);
            pixel.rgb = pixel.rgb.map(|value| value.min(1.0));
            pixel.opacity = pixel.opacity.min(1.0);
        }
    }
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
