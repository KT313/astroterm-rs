//! The terminal backend: the renderer that shows the sky as characters, and below it the session lifetime,
//! presenting canvases, and input with its key bindings.

mod dispatch;
mod input;
mod pixels;
pub use dispatch::Renderer;
pub mod graphics;
mod keys;
mod present;
mod renderer;
mod memory;
mod session;

pub use input::{FrameInput, poll_frame_input};
pub use keys::{format_key_bindings_help, key_to_control};
pub use present::{detect_cell_aspect_ratio, fit_square_viewport};
pub use renderer::open_terminal_renderer;
pub use session::{TerminalSession, open_terminal_session};
