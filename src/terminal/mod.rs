//! Terminal entry points: input, renderer lifetime, and completed-frame presentation.
//! The frame sequence is in `pipeline.rs`; supporting rendering and transport modules stay private.
mod pipeline;
mod rendering;
mod transport;
mod input;
mod diagnostics;

pub use rendering::{Renderer, open_terminal_renderer};
pub use transport::{TerminalSession, open_terminal_session, detect_cell_aspect_ratio, fit_square_viewport};
pub use input::{FrameInput, poll_frame_input, format_key_bindings_help, key_to_control};
pub use transport::graphics::{IMAGE_ID, compose_halfblocks, compose_image, encode_image, present_frame,
    serialize_frame, serialize_frame_into};
pub use transport::graphics::kitty::{IMAGE_IDS as KITTY_IMAGE_IDS, other_image_id as other_kitty_image_id,
    encode_upload as encode_kitty_upload, encode_upload_into as encode_kitty_upload_into,
    serialize_swap as serialize_kitty_swap, serialize_swap_into as serialize_kitty_swap_into};
