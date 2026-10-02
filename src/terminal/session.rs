//! The terminal session: raw mode and the alternate screen for as long as the application runs.

use std::io::{self, BufWriter, Stdout, Write};
use std::sync::Once;

use crossterm::cursor::{Hide, Show};
use crossterm::style::ResetColor;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use crate::canvas::Canvas;

use super::present::{Frame, Presenter, detect_cell_aspect_ratio, fit_square_viewport};

/// An open terminal session. The terminal is restored when this is dropped, including during a panic.
pub struct TerminalSession {
    out: BufWriter<Stdout>,
    presenter: Presenter,
}

/// Switch the terminal to raw mode and the alternate screen, with a hidden cursor.
pub fn open_terminal_session() -> io::Result<TerminalSession> {
    install_panic_restore_hook();
    terminal::enable_raw_mode()?;
    let mut session = TerminalSession {
        out: BufWriter::new(io::stdout()),
        presenter: Presenter::default(),
    };
    execute!(session.out, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
    Ok(session)
}

impl TerminalSession {
    /// Size the canvases of a frame to the current terminal: a square, centered sky canvas and, with `with_panel`, an
    /// (initially empty) panel for the top left corner. `aspect_ratio` (cell height / width) overrides the detected
    /// one. The screen is cleared, so the next frame is drawn in full.
    pub fn fit_frame(&mut self, aspect_ratio: Option<f64>, with_panel: bool) -> io::Result<Frame> {
        // layout
        let (columns, rows) = terminal::size()?;
        let aspect_ratio = aspect_ratio.unwrap_or_else(detect_cell_aspect_ratio);
        let viewport = fit_square_viewport(rows, columns, aspect_ratio);

        // start over on a blank screen
        queue!(self.out, ResetColor, Clear(ClearType::All))?;
        self.presenter.reset(rows, columns, viewport);
        let panel = with_panel.then(|| Canvas::new(0, 0));
        Ok(Frame {
            sky: Canvas::new(viewport.height, viewport.width),
            panel,
        })
    }

    /// Show the frame, writing only the cells that changed since the last one.
    pub fn present(&mut self, frame: &Frame) -> io::Result<()> {
        self.presenter.present(&mut self.out, frame)?;
        self.out.flush()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.out.flush();
        restore_terminal();
    }
}

/// Leave the alternate screen and raw mode. Safe to call more than once.
fn restore_terminal() {
    let _ = execute!(io::stdout(), ResetColor, Show, LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

/// Restore the terminal before the panic message is printed, so it is not lost on the alternate screen.
fn install_panic_restore_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous_hook(info);
        }));
    });
}
