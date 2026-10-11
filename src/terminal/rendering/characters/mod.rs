//! Character renderer setup and named frame stages.
mod setup;
mod steps;

pub use setup::open_terminal_renderer;
pub(in crate::terminal) use setup::{character_viewport, fit_character_terminal};
pub(in crate::terminal) use steps::{prepare_character_timezone, prepare_character_timing_fields, rasterize_character_sky, draw_character_notice, draw_character_panel, present_character_frame};
