//! Pixel renderer setup and named frame stages; text and protocol detection stay separate.
mod setup;
mod steps;
mod layout;
mod detection;
mod text;

pub(in crate::terminal) use setup::{open_pixel_renderer, pixel_viewport, fit_pixel_terminal};
pub(in crate::terminal) use steps::{prepare_pixel_timezone, initialize_pixel_canvas, rasterize_pixel_sky, compose_pixel_sky,
    prepare_pixel_fields, layout_pixel_text, prepare_pixel_glyphs, paint_pixel_text, encode_pixel_cells,
    serialize_pixel_cells, present_pixel_cells, convert_kitty_pixels, encode_kitty_upload, serialize_kitty_swap,
    upload_and_swap_kitty_image};
