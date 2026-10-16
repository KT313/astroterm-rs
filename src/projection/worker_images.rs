//! Worker-owned image preparation and completion-order handoff. Vec ownership moves through the channel;
//! no pixels are copied, and no worker can write an image after handing it back to the caller.
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc;
use rayon::ThreadPool;
use crate::model::Texel;
use crate::observation::Observation;
use crate::time::StepTimes;
use super::pixel_warp::Screen;
use super::region_drawing::{SelectedTile, draw_regions};
use super::screen_blend::blend_screen_images;

struct CompletedImage {
    slot: usize,
    pixels: Vec<Texel>,
    drawing_result: std::thread::Result<()>,
}

pub(super) fn prepare_image(pixels: &mut Vec<Texel>, pixel_count: usize) {
    // Resize initializes new pixels. Clear only the retained prefix to avoid clearing new pixels twice.
    let retained = pixels.len().min(pixel_count);
    pixels.resize(pixel_count, Texel::default());
    pixels[..retained].fill(Texel::default());
}

pub(super) fn draw_worker_images(pool: &ThreadPool, chunks: &[&[SelectedTile<'_>]], buffers: &mut [Vec<Texel>], pixel_count: usize, screen: &Screen, observation: &Observation, times: &mut StepTimes) -> usize {
    assert!(!chunks.is_empty(), "projection needs at least one drawing job");
    assert!(buffers.len() >= chunks.len(), "each projection job needs its own image slot");

    // A bounded channel can hold every result even if composition or unwinding delays the receiver.
    let (sender, receiver) = mpsc::sync_channel(chunks.len());
    let result = pool.in_place_scope(|scope| {
        for (slot, (&tiles, buffer)) in chunks.iter().zip(buffers.iter_mut()).enumerate() {
            let sender = sender.clone();
            let mut pixels = std::mem::take(buffer);
            scope.spawn(move |_| {
                // Return the allocation even after a drawing panic; the caller drains all results first.
                let drawing_result = catch_unwind(AssertUnwindSafe(|| {
                    prepare_image(&mut pixels, pixel_count);
                    draw_regions(&mut pixels, tiles, screen, observation, &mut StepTimes::default());
                }));
                let completed = CompletedImage { slot, pixels, drawing_result };
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

fn collect_images(receiver: mpsc::Receiver<CompletedImage>, buffers: &mut [Vec<Texel>], times: &mut StepTimes) -> std::thread::Result<usize> {
    let mut accumulated: Option<CompletedImage> = None;
    let mut failure = None;
    loop {
        // Wait only for the next completed job, not for all jobs or a particular region order.
        times.measure("Projection.Worker wait");
        let Ok(completed) = receiver.recv() else { break; };
        let CompletedImage { slot, pixels, drawing_result } = completed;
        if let Err(panic) = drawing_result {
            if failure.is_none() { failure = Some(panic); }
            buffers[slot] = pixels;
            continue;
        }

        // After an error, keep collecting allocations without presenting a partial image.
        if failure.is_some() {
            buffers[slot] = pixels;
            continue;
        }
        if let Some(first) = &mut accumulated {
            times.measure("Projection.Worker composition");
            blend_screen_images(&mut first.pixels, &pixels);
            buffers[slot] = pixels;
        } else {
            accumulated = Some(CompletedImage { slot, pixels, drawing_result: Ok(()) });
        }
    }

    // The first completed image remains the accumulator and goes back into its original reusable slot.
    let result_slot = accumulated.map(|image| {
        buffers[image.slot] = image.pixels;
        image.slot
    });
    if let Some(panic) = failure { return Err(panic); }
    Ok(result_slot.expect("projection workers returned no image"))
}

#[cfg(test)]
#[path = "worker_images_tests.rs"]
mod tests;
