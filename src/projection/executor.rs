//! Optional projection-only worker pool. Every task runs the same independent region loop into its own
//! image. The caller screen-blends results as they arrive and retains all image allocations for reuse.
use std::{io, ops::Range};
use rayon::{ThreadPool, ThreadPoolBuilder};
use crate::model::{ProjectionKind, Region, Texel, View};
use crate::observation::Observation;
use crate::time::StepTimes;
use super::prepare_camera;
use super::options::ProjectionOptions;
use super::pixel_warp::Screen;
use super::region_drawing::{draw_regions, estimate_stereographic_work, select_region_tiles};
use super::worker_image::WorkerImage;
use super::worker_images::{draw_worker_images, prepare_image};

/// Lives outside the frame loop. Zero workers runs the region loop locally; positive counts use a
/// dedicated pool, never Rayon's global pool. Image capacity is retained; each job clears old contents.
pub struct ProjectionExecutor {
    options: ProjectionOptions,
    pool: Option<ThreadPool>,
    screen: Option<Screen>,
    worker_times: Vec<StepTimes>,           // borrowed by scoped tasks; never moved with completed images
    work_weights: Vec<f64>,
    region_chunks: Vec<Range<usize>>,
    buffers: Vec<WorkerImage>,                  // one reusable image slot per worker, or one for direct execution
}

impl Default for ProjectionExecutor {
    fn default() -> Self {
        Self {
            options: ProjectionOptions::default(),
            pool: None,
            screen: None,
            worker_times: Vec::new(),
            work_weights: Vec::new(),
            region_chunks: Vec::new(),
            buffers: vec![WorkerImage::default()],
        }
    }
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
        Ok(Self {
            options: ProjectionOptions::default(),
            pool: Some(pool),
            screen: None,
            worker_times: (0..workers).map(|_| StepTimes::default()).collect(),
            work_weights: Vec::new(),
            region_chunks: Vec::new(),
            buffers: (0..workers).map(|_| WorkerImage::default()).collect(),
        })
    }

    pub(super) fn project(&mut self, size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, times: &mut StepTimes) -> &[Texel] {
        // Degenerate drawing areas need a correctly sized transparent image, but no worker jobs.
        let pixel_count = size.checked_mul(size).expect("drawing area pixel count overflow");
        if size < 2 {
            prepare_image(&mut self.buffers[0].pixels, pixel_count);
            self.buffers[0].invalidate();
            return &self.buffers[0].pixels;
        }
        times.measure("Projection.Camera preparation");
        let camera = prepare_camera(view);
        let screen = self.screen.get_or_insert_with(|| Screen::new(camera, size));
        screen.update(camera, size);
        times.measure("Projection.Tile selection");
        let tiles = select_region_tiles(regions, selected);

        // Direct execution uses exactly the same loop as each worker, with the normal frame timer.
        let Some(pool) = &self.pool else {
            times.measure("Projection.Star buffer preparation");
            prepare_image(&mut self.buffers[0].pixels, pixel_count);
            self.buffers[0].invalidate();
            let image = &mut self.buffers[0];
            draw_regions::<false>(&mut image.pixels, &mut image.touched, &tiles, screen, observation, &self.options, times);
            times.measure("Projection");
            return &self.buffers[0].pixels;
        };

        // Estimate only when stereographic rendering has multiple tasks. Other paths keep equal counts.
        let workers = pool.current_num_threads();
        if tiles.is_empty() {
            self.region_chunks.clear();
            self.region_chunks.push(0..0); // One worker clears the result for an empty selection.
        } else if workers > 1 && tiles.len() > 1 && view.projection == ProjectionKind::Stereographic {
            times.measure("Projection.Work estimation");
            estimate_stereographic_work(&mut self.work_weights, &tiles, &observation.patch.center, camera.scale, size, view.fov_degrees);
            split_regions_by_work(&mut self.region_chunks, &self.work_weights, workers);
        } else {
            split_region_chunks(&mut self.region_chunks, tiles.len(), workers);
        }

        // Workers resize and clear their own images. The caller merges completions while others draw.
        times.measure("Projection.Worker dispatch");
        let result_slot = draw_worker_images(pool, &tiles, &self.region_chunks, &mut self.buffers, &mut self.worker_times, size, screen, observation, &self.options, times);
        times.measure("Projection");
        &self.buffers[result_slot].pixels
    }
}

/// Fill reusable index ranges; jobs borrow only the tile slice named by their range.
fn split_region_chunks(chunks: &mut Vec<Range<usize>>, region_count: usize, workers: usize) {
    assert!(workers > 0);
    chunks.clear();
    let count = workers.min(region_count);
    if count == 0 { return; }

    // Give the first few tasks one extra region when the division has a remainder.
    let base = region_count / count;
    let extra = region_count % count;
    let mut first = 0;
    for index in 0..count {
        let length = base + usize::from(index < extra);
        chunks.push(first..first + length);
        first += length;
    }
}

/// Balance approximate work using consecutive ranges, retaining the original scheduling decisions.
fn split_regions_by_work(chunks: &mut Vec<Range<usize>>, work: &[f64], workers: usize) {
    assert!(workers > 0);
    chunks.clear();
    let count = workers.min(work.len());
    let total_work: f64 = work.iter().sum();
    if count <= 1 || total_work == 0.0 {
        split_region_chunks(chunks, work.len(), workers);
        return;
    }

    // Cut at cumulative work targets while reserving one region per remaining task.
    let mut first = 0;
    let mut completed_work = 0.0;
    for (index, &cost) in work.iter().enumerate() {
        let remaining_chunks = count - chunks.len() - 1;
        if remaining_chunks == 0 { break; }
        completed_work += cost;
        let target = total_work * ((chunks.len() + 1) as f64 / count as f64);
        let remaining_regions = work.len() - index - 1;
        if completed_work >= target || remaining_regions == remaining_chunks {
            chunks.push(first..index + 1);
            first = index + 1;
        }
    }

    // Include the final suffix, even when it contains only cheap regions.
    chunks.push(first..work.len());
}

/// Own the direct result in headless fixtures without adding a pixel copy to their benchmarks.
#[cfg(test)]
pub(super) fn project_test_image(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, times: &mut StepTimes) -> Vec<Texel> {
    let mut executor = ProjectionExecutor::default();
    executor.project(size, regions, selected, observation, view, times);
    std::mem::take(&mut executor.buffers[0].pixels)
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
