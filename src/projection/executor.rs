//! Optional projection-only worker pool. Every task runs the same independent region loop into its own
//! image. The caller screen-blends results as they arrive and retains all image allocations for reuse.
use std::io;
use rayon::{ThreadPool, ThreadPoolBuilder};
use crate::model::{ProjectionKind, Region, Texel, View};
use crate::observation::Observation;
use crate::time::StepTimes;
use super::prepare_camera;
use super::pixel_warp::Screen;
use super::region_drawing::{draw_regions, estimate_stereographic_work, select_region_tiles};
use super::worker_images::{draw_worker_images, prepare_image};

/// Lives outside the frame loop. Zero workers runs the region loop locally; positive counts use a
/// dedicated pool, never Rayon's global pool. Image capacity is retained; each job clears old contents.
pub struct ProjectionExecutor {
    pool: Option<ThreadPool>,
    buffers: Vec<Vec<Texel>>,                  // one reusable image slot per worker, or one for direct execution
}

impl Default for ProjectionExecutor {
    fn default() -> Self { Self { pool: None, buffers: vec![Vec::new()] } }
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
        Ok(Self { pool: Some(pool), buffers: (0..workers).map(|_| Vec::new()).collect() })
    }

    pub(super) fn project(&mut self, size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, times: &mut StepTimes) -> &[Texel] {
        // Degenerate drawing areas need a correctly sized transparent image, but no worker jobs.
        let pixel_count = size.checked_mul(size).expect("drawing area pixel count overflow");
        if size < 2 {
            prepare_image(&mut self.buffers[0], pixel_count);
            return &self.buffers[0];
        }
        times.measure("Projection.Camera preparation");
        let camera = prepare_camera(view);
        let screen = Screen::new(camera, size);
        times.measure("Projection.Tile selection");
        let tiles = select_region_tiles(regions, selected);

        // Direct execution uses exactly the same loop as each worker, with the normal frame timer.
        let Some(pool) = &self.pool else {
            times.measure("Projection.Star buffer preparation");
            prepare_image(&mut self.buffers[0], pixel_count);
            draw_regions(&mut self.buffers[0], &tiles, &screen, observation, times);
            times.measure("Projection");
            return &self.buffers[0];
        };

        // Estimate only when stereographic rendering has multiple tasks. Other paths keep equal counts.
        let workers = pool.current_num_threads();
        let chunks = if tiles.is_empty() {
            vec![tiles.as_slice()] // One worker clears the result when the selection becomes empty.
        } else if workers > 1 && tiles.len() > 1 && view.projection == ProjectionKind::Stereographic {
            times.measure("Projection.Work estimation");
            let work = estimate_stereographic_work(&tiles, &observation.patch.center, camera.scale, size, view.fov_degrees);
            split_regions_by_work(&tiles, &work, workers)
        } else {
            split_region_chunks(&tiles, workers)
        };

        // Workers resize and clear their own images. The caller merges completions while others draw.
        times.measure("Projection.Worker dispatch");
        let result_slot = draw_worker_images(pool, &chunks, &mut self.buffers, pixel_count, &screen, observation, times);
        times.measure("Projection");
        &self.buffers[result_slot]
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

/// Balance approximate work using consecutive slices without splitting individual regions. Boundaries follow
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

/// Own the direct result in headless fixtures without adding a pixel copy to their benchmarks.
#[cfg(test)]
pub(super) fn project_test_image(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, times: &mut StepTimes) -> Vec<Texel> {
    let mut executor = ProjectionExecutor::default();
    executor.project(size, regions, selected, observation, view, times);
    std::mem::take(&mut executor.buffers[0])
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
