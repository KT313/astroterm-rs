use astroterm::canvas::Canvas;
use astroterm::model::{ObservedSky, View, ProjectionViewport, RenderOptions};
use astroterm::projection::borrow_projected;
use astroterm::state::ProjectionCache;

fn mutate_before_rendering(sky: &ObservedSky, cache: &mut ProjectionCache, canvas: &mut Canvas, options: &RenderOptions) {
    let projected = borrow_projected(cache, sky, &View::default(), ProjectionViewport { width: 8, height: 8 });
    cache.invalidate_view(); // cached geometry remains borrowed until rendering finishes
    astroterm::scene::draw_sky_scene(canvas, options, &projected);
}

fn main() {}
