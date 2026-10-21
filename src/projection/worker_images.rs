//! Worker-owned image preparation and completion-order handoff. Pixels and block tracking move together;
//! no pixels are copied, and no worker can write an image after handing it back to the caller.
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc;
use rayon::ThreadPool;
use crate::model::Texel;
use crate::observation::Observation;
use crate::time::StepTimes;
use super::pixel_warp::Screen;
use super::options::ProjectionOptions;
use super::region_drawing::{SelectedTile, draw_regions};
use super::worker_image::WorkerImage;

struct CompletedImage {
    slot: usize,
    image: WorkerImage,
    drawing_result: std::thread::Result<()>,
}

pub(super) fn prepare_image(pixels: &mut Vec<Texel>, pixel_count: usize) {
    // Resize initializes new pixels. Clear only the retained prefix to avoid clearing new pixels twice.
    let retained = pixels.len().min(pixel_count);
    pixels.resize(pixel_count, Texel::default());
    pixels[..retained].fill(Texel::default());
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_worker_images(pool: &ThreadPool, tiles: &[SelectedTile<'_>], chunks: &[std::ops::Range<usize>], buffers: &mut [WorkerImage], worker_times: &mut [StepTimes], size: usize, screen: &Screen, observation: &Observation, options: &ProjectionOptions, times: &mut StepTimes) -> usize {
    assert!(!chunks.is_empty(), "projection needs at least one drawing job");
    assert!(buffers.len() >= chunks.len(), "each projection job needs its own image slot");
    assert!(worker_times.len() >= chunks.len(), "each projection job needs its own timer");

    // A bounded channel can hold every result even if composition or unwinding delays the receiver.
    let (sender, receiver) = mpsc::sync_channel(chunks.len());
    let result = pool.in_place_scope(|scope| {
        for (slot, ((range, buffer), timer)) in chunks.iter().zip(buffers.iter_mut()).zip(worker_times).enumerate() {
            let tiles = &tiles[range.clone()];
            let sender = sender.clone();
            let mut image = std::mem::take(buffer);
            scope.spawn(move |_| {
                // Return the allocation even after a drawing panic; the caller drains all results first.
                let drawing_result = catch_unwind(AssertUnwindSafe(|| {
                    image.prepare(size);
                    timer.begin_frame();
                    draw_regions::<true>(&mut image.pixels, &mut image.touched, tiles, screen, observation, options, timer);
                    timer.measure("END");
                }));
                if drawing_result.is_err() {
                    image.invalidate();
                    timer.reset_frame_timings(); // Discard an unfinished interval before reusing the worker.
                }
                let completed = CompletedImage { slot, image, drawing_result };
                let _ = sender.send(completed); // A disconnected receiver means the caller is already unwinding.
            });
        }

        // Only jobs retain senders now: failures cannot leave the receiver waiting for a nonexistent result.
        drop(sender);
        collect_images(receiver, buffers, times)
    });

    // The scope has joined every job before a worker's original panic is re-raised.
    match result {
        Ok(slot) => slot,
        Err(panic) => resume_unwind(panic),
    }
}

fn collect_images(receiver: mpsc::Receiver<CompletedImage>, buffers: &mut [WorkerImage], times: &mut StepTimes) -> std::thread::Result<usize> {
    let mut accumulated: Option<CompletedImage> = None;
    let mut failure = None;
    loop {
        // Wait only for the next completed job, not for all jobs or a particular region order.
        times.measure("Projection.Worker wait");
        let Ok(completed) = receiver.recv() else { break; };
        let CompletedImage { slot, image, drawing_result } = completed;
        if let Err(panic) = drawing_result {
            if failure.is_none() { failure = Some(panic); }
            buffers[slot] = image;
            continue;
        }

        // After an error, keep collecting allocations without presenting a partial image.
        if failure.is_some() {
            buffers[slot] = image;
            continue;
        }
        if let Some(first) = &mut accumulated {
            times.measure("Projection.Worker composition");
            first.image.merge(&image);
            buffers[slot] = image;
        } else {
            accumulated = Some(CompletedImage { slot, image, drawing_result: Ok(()) });
        }
    }

    // The first completed image remains the accumulator and goes back into its original reusable slot.
    let result_slot = accumulated.map(|image| {
        buffers[image.slot] = image.image;
        image.slot
    });
    if let Some(panic) = failure { return Err(panic); }
    Ok(result_slot.expect("projection workers returned no image"))
}

#[cfg(test)]
#[path = "worker_images_tests.rs"]
mod tests;
