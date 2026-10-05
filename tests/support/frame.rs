//! Existing rendering fixtures use this adapter to exercise projection and drawing as separate stages.
use astroterm::canvas::Canvas;
use astroterm::model::Sky;
use astroterm::model::projection::{ProjectionViewport as Viewport, View};
use astroterm::model::rendering::RenderOptions;
use astroterm::projection::project_sky;
pub fn draw_sky_scene(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    let projected_data = project_sky(
        sky,
        view,
        Viewport {
            height: canvas.height(),
            width: canvas.width(),
        },
    );
    let projected = projected_data.view(sky);
    astroterm::scene::draw_sky_scene(canvas, options, &projected);
}
