//! State-owned whole-sky caches. Metadata and terminal presentation are assembled after these immutable results.
mod keys;
mod production;
pub(super) use production::draw_prepared_pixels;
pub(in crate::scene) use keys::prepare_pixel_star_inputs;
use crate::timing::{BufferId, Operation};
use super::diagnostics::memory::{describe_candidate, describe_canvas, record_scene_candidate, record_scene_commit};

#[cfg(test)]
mod tests;
#[cfg(test)]
use crate::cache::CacheConfig;

use crate::state::SceneCache;
use crate::model::{RenderOptions};
use super::draw_sky_scene_with_times;
#[cfg(test)]
use super::raster::pixels::draw_pixel_sky;
use crate::cache::Group;
use crate::canvas::Canvas;
use crate::model::ProjectedSky;
use crate::timing::StepTimes;
use keys::{capture_star_keys, clear_scene_candidate, describe_star_keys};

use crate::model::{SceneKey, StarKeys};
fn capture_scene_key(
    candidate: &mut Option<SceneKey>,
    sky: &ProjectedSky<'_>,
    options: RenderOptions,
    canvas_size: Option<(usize, usize)>,
) {
    let key = candidate.get_or_insert_with(|| SceneKey {
        production: None,
        stars: StarKeys::Pixels(Vec::new()),
        planets: Vec::new(), moon: None, constellations: Vec::new(), horizon: Vec::new(), labels: Vec::new(),
        pixel_fov_degrees: canvas_size.is_none().then_some(sky.fov_degrees),
        viewport: sky.viewport, facing: sky.facing, warning: sky.outside_accuracy_range, brightness_warning: sky.magnitude_clipping().any(), options, canvas_size,
    });
    clear_scene_candidate(key); // also resets a candidate left behind by a failed image allocation
    key.production = None; // editable inputs always use exact structural comparison
    capture_star_keys(&mut key.stars, sky, &options, canvas_size.is_some());
    key.planets.extend_from_slice(sky.planets);
    key.moon = sky.moon.cell.map(|_| (*sky.moon).clone());
    key.constellations.extend_from_slice(sky.constellations);
    key.horizon.extend_from_slice(sky.horizon);
    key.labels.extend_from_slice(sky.horizon_labels);
    key.viewport = sky.viewport;
    key.pixel_fov_degrees = canvas_size.is_none().then_some(sky.fov_degrees); // a centered star can brighten without moving to another pixel
    key.facing = sky.facing;
    key.warning = sky.outside_accuracy_range;
    key.brightness_warning = sky.magnitude_clipping().any();
    key.options = options;
    key.canvas_size = canvas_size;
}


pub(super) fn prepare_pixel_candidate(storage: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) -> bool {
    let candidate_before = times.inspect_memory(|| storage.pixel_candidate.as_ref().map(describe_candidate)).flatten();
    times.measure("Raster cache key", || {
        capture_scene_key(&mut storage.pixel_candidate, sky, *options, None)
    });
    let key = storage.pixel_candidate.as_ref().expect("raster candidate captured");
    record_scene_candidate(times, BufferId::PixelCandidate, candidate_before, key);
    times.describe("Raster cache key", || describe_star_keys(&key.stars, sky.stars.len()));
    let refresh = times.measure("Raster cache decision", || {
        storage.pixels
            .needs_refresh(key, epoch, None, storage.config.allows(Group::Raster))
    });
    times.record_candidate_decision(times.last_memory_step(), BufferId::PixelCandidate, BufferId::PixelScene, refresh, storage.pixels.stats.last_reason);
    refresh
}

pub(super) fn refresh_pixel_scene(storage: &mut SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) -> Option<()> {
    let key = storage.pixel_candidate.as_ref().expect("raster candidate captured");
    let StarKeys::Pixels(stars) = &key.stars else { unreachable!("pixel drawing requires pixel inputs") };
    let image = super::pipeline::draw_pixel_sky_from_inputs(&mut storage.star_layer, &mut storage.image_scratch, sky, options, times, stars.iter().copied())?;
    let outcome = times.measure("Raster cache store", || {
        let key = storage.pixel_candidate.take().expect("raster candidate captured");
        let (outcome, displaced) = storage.pixels.store_displacing(key, epoch, 0.0, image);
        recycle_displaced_pixels(storage, displaced);
        outcome
    });
    record_scene_commit(times, BufferId::PixelCandidate, BufferId::PixelScene, outcome);
    Some(())
}

/// Keep the allocations a pixel store displaced: the old key becomes the next (cleared) candidate and the old
/// image's bytes become the next canvas.
pub(super) fn recycle_displaced_pixels(storage: &mut SceneCache, displaced: Option<(SceneKey, image::RgbaImage)>) {
    let Some((mut key, image)) = displaced else { return; };
    clear_scene_candidate(&mut key);
    storage.pixel_candidate = Some(key);
    storage.image_scratch = image.into_raw();
}

pub(super) fn clear_pixel_candidate(storage: &mut SceneCache, times: &mut StepTimes) {
    let cleared_before = times.inspect_memory(|| describe_candidate(storage.pixel_candidate.as_ref().unwrap()));
    times.measure("Raster candidate clear", || {
        clear_scene_candidate(storage.pixel_candidate.as_mut().expect("raster candidate captured"));
    });
    times.record_shape(BufferId::PixelCandidate, Operation::Clear, cleared_before, || describe_candidate(storage.pixel_candidate.as_ref().unwrap()));
}

pub(super) fn prepare_character_candidate(storage: &mut SceneCache, canvas: &Canvas, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) -> bool {
    let candidate_before = times.inspect_memory(|| storage.character_candidate.as_ref().map(describe_candidate)).flatten();
    times.measure("Raster cache key", || {
        capture_scene_key(
            &mut storage.character_candidate,
            sky,
            *options,
            Some((canvas.height(), canvas.width())),
        )
    });
    let key = storage.character_candidate.as_ref().expect("raster candidate captured");
    record_scene_candidate(times, BufferId::CharacterCandidate, candidate_before, key);
    times.describe("Raster cache key", || describe_star_keys(&key.stars, sky.stars.len()));
    let refresh = times.measure("Raster cache decision", || {
        storage.characters
            .needs_refresh(key, epoch, None, storage.config.allows(Group::Raster))
    });
    times.record_candidate_decision(times.last_memory_step(), BufferId::CharacterCandidate, BufferId::CharacterScene, refresh, storage.characters.stats.last_reason);
    refresh
}

pub(super) fn refresh_character_scene(storage: &mut SceneCache, canvas: &mut Canvas, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) {
    draw_sky_scene_with_times(canvas, options, sky, times);
    let copy = times.measure("Character cache copy", || canvas.clone());
    times.record_shape(BufferId::CharacterScene, Operation::Copy, None, || describe_canvas(canvas));
    let outcome = times.measure("Raster cache store", || {
        let key = storage.character_candidate.take().expect("raster candidate captured");
        storage.characters.store(key, epoch, 0.0, copy)
    });
    record_scene_commit(times, BufferId::CharacterCandidate, BufferId::CharacterScene, outcome);
}

pub(super) fn reuse_character_scene(storage: &mut SceneCache, canvas: &mut Canvas, times: &mut StepTimes) {
    times.measure("Raster output copy", || canvas.clone_from(storage.characters.value()));
    times.record_shape(BufferId::CharacterFrame, Operation::Copy, None, || describe_canvas(canvas));
    let cleared_before = times.inspect_memory(|| describe_candidate(storage.character_candidate.as_ref().unwrap()));
    times.measure("Raster candidate clear", || {
        clear_scene_candidate(storage.character_candidate.as_mut().expect("raster candidate captured"));
    });
    times.record_shape(BufferId::CharacterCandidate, Operation::Clear, cleared_before, || describe_candidate(storage.character_candidate.as_ref().unwrap()));
}
