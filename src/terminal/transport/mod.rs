//! Terminal lifetime, the sky layout on the screen and the completed-frame transport.
mod session;
mod layout;
pub(crate) mod graphics;

pub use session::{TerminalSession, open_terminal_session};
pub use layout::{Viewport, detect_cell_aspect_ratio, fit_square_viewport};
