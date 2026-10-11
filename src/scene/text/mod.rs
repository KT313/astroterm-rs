//! Text rasterization and metadata panel drawing.
mod panel;
mod raster;
pub use panel::draw_metadata_panel;
pub use raster::{begin_text_frame, create_text_rasterizer, set_text_cell_size, paint_text_buffer, draw_text};
pub(crate) use raster::paint_text_buffer_with_times;
