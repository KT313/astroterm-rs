//! Existing rendering fixtures use this adapter to exercise projection and drawing as separate stages.
use astroterm::{
    canvas::Canvas,
    projection::{View, Viewport, project_sky},
    scene::RenderOptions,
    sky::Sky,
};
pub fn draw_sky_scene(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    let projected = project_sky(
        sky,
        view,
        Viewport {
            height: canvas.height(),
            width: canvas.width(),
        },
    );
    astroterm::scene::draw_sky_scene(canvas, options, &projected);
}
