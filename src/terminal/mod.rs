//! Terminal I/O that the frame loop and the startup use: the startup progress display, the raw-mode session,
//! the key and resize events, and the graphics transport (encoders and protocol detection) step 7 writes through.
mod startup;
pub use startup::{open_startup_progress, StartupProgress};
pub(crate) mod transport;
mod input;

pub use transport::{TerminalSession, Viewport, open_terminal_session, detect_cell_aspect_ratio, fit_square_viewport};
pub use input::{FrameInput, poll_frame_input, format_key_bindings_help, key_to_control};
pub use transport::graphics::{IMAGE_ID, compose_image, encode_image, present_frame, serialize_frame, serialize_frame_into};
pub use transport::graphics::kitty::{IMAGE_IDS as KITTY_IMAGE_IDS, other_image_id as other_kitty_image_id,
    encode_upload as encode_kitty_upload, encode_upload_into as encode_kitty_upload_into,
    serialize_swap as serialize_kitty_swap, serialize_swap_into as serialize_kitty_swap_into};
