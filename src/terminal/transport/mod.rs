//! Terminal lifetime and completed-frame transport.
mod session;
mod present;
pub(crate) mod graphics;

pub use session::{TerminalSession, open_terminal_session};
pub use present::{detect_cell_aspect_ratio, fit_square_viewport};
