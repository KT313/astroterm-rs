//! Keyboard and resize events between frames. What keys do (other than quitting) is up to the caller.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// What happened since the last frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameInput {
    /// A quit key was pressed.
    pub quit: bool,
    /// The terminal was resized.
    pub resized: bool,
    /// Other key presses (including auto-repeats), in order.
    pub keys: Vec<KeyEvent>,
}

/// Drain pending terminal events without blocking. Keys after a quit key are dropped.
pub fn poll_frame_input(quit_on_any_key: bool) -> io::Result<FrameInput> {
    let mut input = FrameInput::default();
    while event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press && is_quit_key(&key, quit_on_any_key) => {
                input.quit = true;
                return Ok(input);
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => input.keys.push(key),
            Event::Resize(..) => input.resized = true,
            _ => {}
        }
    }
    Ok(input)
}

/// `q`, Esc and Ctrl-C quit (raw mode turns Ctrl-C into a key press), or any key with `quit_on_any_key`.
fn is_quit_key(key: &KeyEvent, quit_on_any_key: bool) -> bool {
    let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    quit_on_any_key || ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_keys() {
        let key = |code, modifiers| KeyEvent::new(code, modifiers);
        assert!(is_quit_key(&key(KeyCode::Char('q'), KeyModifiers::NONE), false));
        assert!(is_quit_key(&key(KeyCode::Esc, KeyModifiers::NONE), false));
        assert!(is_quit_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false));
        assert!(!is_quit_key(&key(KeyCode::Char('c'), KeyModifiers::NONE), false));
        assert!(is_quit_key(&key(KeyCode::Char('x'), KeyModifiers::NONE), true));
    }
}
