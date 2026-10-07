//! Draw complete scenes in a fixed order. Supporting raster and diagnostic details stay below this module.
use crate::canvas::Canvas;
use crate::model::{ProjectedSky, PreparedScene, RenderOptions};
use crate::timing::StepTimes;
use super::raster::{draw_orientation_labels, draw_coverage_notice};
use super::{draw_horizon_line, draw_constellations, draw_planets, draw_moon};
use super::diagnostics::memory::{record_character_initialization, record_character_stars};

use super::raster::pixels::{
    initialize_pixel_canvas, draw_pixel_horizon, draw_pixel_stars, draw_pixel_constellations, draw_pixel_planets,
    draw_pixel_moon, draw_pixel_grid,
};
use super::diagnostics::memory::{
    record_pixel_horizon, record_pixel_stars, record_pixel_constellations, record_pixel_planets,
    record_pixel_initialization, record_pixel_finalization,
};

/// Reuse or redraw the pixel scene, then lend the completed immutable image without copying pixels.
pub fn draw_pixels<'a>(storage: &'a mut crate::state::SceneCache, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) -> Option<&'a image::RgbaImage> {
    let refresh = super::caching::prepare_pixel_candidate(storage, sky, options, epoch, times); // compare this frame's drawing inputs with the saved image
    if refresh { super::caching::refresh_pixel_scene(storage, sky, options, epoch, times)?; }   // redraw and store the image when its inputs changed
    else { super::caching::clear_pixel_candidate(storage, times); }                             // keep candidate capacity for the next comparison
    Some(storage.pixel_image()) // borrow the original image; callers needing ownership must explicitly copy
}

pub fn draw_characters(storage: &mut crate::state::SceneCache, canvas: &mut Canvas, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64) {
    draw_characters_with_times(storage, canvas, sky, options, epoch, &mut StepTimes::default());
}

pub(crate) fn draw_characters_with_times(storage: &mut crate::state::SceneCache, canvas: &mut Canvas, sky: &ProjectedSky<'_>, options: &RenderOptions, epoch: f64, times: &mut StepTimes) {
    let refresh = super::caching::prepare_character_candidate(storage, canvas, sky, options, epoch, times); // check whether the saved character scene still matches
    if refresh { super::caching::refresh_character_scene(storage, canvas, sky, options, epoch, times); }  // redraw and save the character cells
    else { super::caching::reuse_character_scene(storage, canvas, times); }                               // copy saved cells and clear the unused candidate
}

/// Draw the sky as seen in `view` onto the canvas, back to front.
pub fn draw_sky_scene(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    draw_sky_scene_with_times(canvas, options, sky, &mut crate::timing::StepTimes::default());
}

pub(crate) fn draw_sky_scene_with_times(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>, times: &mut StepTimes) {
    draw_sky_scene_prepared(canvas, options, sky, times, None);
}

pub(crate) fn draw_sky_scene_prepared(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>, times: &mut StepTimes, prepared: Option<&PreparedScene>) {
    times.measure("Canvas initialization", || canvas.clear());                              // clear the character cells for a new scene
    record_character_initialization(times, canvas);
    if sky.facing { times.measure("Raster horizon", || draw_horizon_line(canvas, options, sky.horizon)); } // place the horizon behind celestial objects

    times.measure("Raster stars", || super::raster::draw_stars_prepared(canvas, options, sky, prepared)); // draw stars in their prepared brightness order
    record_character_stars(times, canvas, prepared);
    if options.constellations { times.measure("Raster constellations", || draw_constellations(canvas, options, sky)); } // add enabled constellation lines
    times.measure("Raster planets", || draw_planets(canvas, options, sky.planets));          // place the Sun and planets over the stars
    times.measure("Raster moon", || draw_moon(canvas, options, sky));                        // add the Moon's current phase

    times.measure("Orientation labels", || draw_orientation_labels(canvas, options, sky)); // add horizon labels, grid or compass directions
    super::diagnostics::describe_scene(sky, options, times);
    times.measure("Coverage notice", || draw_coverage_notice(canvas, sky));                 // reserve the last row for any date-accuracy warning
    super::diagnostics::describe_coverage_notice(canvas, sky, times);
}

/// Build a pixel image back to front; each calculation retains its own existing timer.
pub(super) fn draw_pixel_sky_with_star_path(sky: &ProjectedSky<'_>, options: &RenderOptions, times: &mut StepTimes, fast_stars: bool, prepared: Option<&PreparedScene>) -> Option<image::RgbaImage> {

    let mut canvas = times.measure("Canvas initialization", || initialize_pixel_canvas(sky.viewport))?; // allocate the sky image and fill its background
    record_pixel_initialization(times, &canvas);
    times.measure("Raster horizon", || draw_pixel_horizon(&mut canvas, sky));                // place the horizon behind celestial objects
    record_pixel_horizon(times, &canvas, sky);

    times.measure("Raster stars", || draw_pixel_stars(&mut canvas, sky, options, fast_stars, prepared)); // draw stars in their prepared brightness order
    record_pixel_stars(times, &canvas, prepared);
    times.measure("Raster constellations", || draw_pixel_constellations(&mut canvas, sky, options)); // add enabled constellation lines
    record_pixel_constellations(times, &canvas, sky);
    times.measure("Raster planets", || draw_pixel_planets(&mut canvas, sky));                // draw the Sun and planets above the stars
    record_pixel_planets(times, &canvas, sky);
    times.measure("Raster moon", || draw_pixel_moon(&mut canvas, sky));                      // draw the Moon with its illuminated shape
    times.measure("Raster grid", || draw_pixel_grid(&mut canvas, sky, options));             // add the requested orientation grid

    let image = times.measure("Raster finalization", || image::RgbaImage::from_raw(canvas.width(), canvas.height(), canvas.take())); // transfer the finished pixels without changing their order
    record_pixel_finalization(times, image.as_ref());
    super::diagnostics::describe_scene(sky, options, times);
    super::diagnostics::describe_minimum_stars(sky, options, fast_stars, times);
    image
}
