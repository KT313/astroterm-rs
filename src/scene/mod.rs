//! Public drawing operations for character scenes, pixel scenes and text. Implementations stay private.
mod caching;
mod diagnostics;
mod pipeline;
mod raster;
mod text;

pub use pipeline::{draw_characters, draw_pixels, draw_sky_scene};
pub use raster::{
    draw_azimuthal_grid, draw_cardinal_directions, draw_constellations, draw_horizon_labels, draw_horizon_line,
    draw_moon, draw_planets, draw_stars, format_star_label, select_moon_appearance, select_planet_appearance,
    select_star_appearance,
};
pub use raster::pixels::draw_pixel_sky;
pub use text::{begin_text_frame, create_text_rasterizer, draw_metadata_panel, draw_text, paint_text_buffer, set_text_cell_size};

pub(crate) use pipeline::draw_characters_with_times;
pub(crate) use diagnostics::memory::describe_canvas;
pub(crate) use pipeline::draw_sky_scene_with_times;
pub(crate) use raster::select_dynamically_named_stars;
pub(crate) use raster::pixels::planet_rgb;
pub(crate) use raster::pixels::star_rgb;
pub(crate) use text::paint_text_buffer_with_times;
