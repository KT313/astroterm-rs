//! The terminal backend: session lifetime, presenting canvases, and input.

mod input;
mod present;
mod session;

pub use input::{FrameInput, poll_frame_input};
pub use present::{Frame, Presenter, Viewport, detect_cell_aspect_ratio, fit_square_viewport};
pub use session::{TerminalSession, open_terminal_session};
