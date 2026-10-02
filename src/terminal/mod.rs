//! The terminal backend: the renderer that shows the sky as characters, and below it the session lifetime,
//! presenting canvases, and input.

mod input;
mod present;
mod renderer;
mod session;

pub use input::{FrameInput, poll_frame_input};
pub use present::{Frame, Presenter, Viewport, detect_cell_aspect_ratio, fit_square_viewport};
pub use renderer::{TerminalRenderer, open_terminal_renderer};
pub use session::{TerminalSession, open_terminal_session};
