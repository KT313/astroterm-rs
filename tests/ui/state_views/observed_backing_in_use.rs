use astroterm::{
    canvas::Canvas,
    model::{ObservedSky, projection::{View, ProjectionViewport}, rendering::RenderOptions},
    projection::borrow_projected,
    state::ProjectionCache,
};

fn mutate_before_rendering(sky: &mut ObservedSky, cache: &ProjectionCache, canvas: &mut Canvas, options: &RenderOptions) {
    let projected = borrow_projected(cache, sky, &View::default(), ProjectionViewport { width: 8, height: 8 });
    sky.stars.clear(); // rendering below still reads these observed records through the projected view
    astroterm::scene::draw_sky_scene(canvas, options, &projected);
}

fn main() {}
