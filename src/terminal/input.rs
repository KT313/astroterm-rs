//! Keyboard and resize events between frames, with keys turned into controls.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use crate::controls::Control;

use super::keys::key_to_control;

/// What happened since the last frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameInput {
    /// Controls triggered by key presses (including auto-repeats), in order. A quit is always last.
    pub controls: Vec<Control>,
    /// The terminal was resized.
    pub resized: bool,
}

/// Drain pending terminal events without blocking. Keys after a quit key are dropped.
pub fn poll_frame_input(quit_on_any_key: bool) -> io::Result<FrameInput> {
    let mut input = FrameInput::default();
    while event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key_to_control(&key, quit_on_any_key) {
                Some(Control::Quit) if key.kind == KeyEventKind::Press => {
                    input.controls.push(Control::Quit);
                    return Ok(input);
                }
                Some(Control::Quit) | None => {} // holding a quit key down doesn't quit
                Some(control) => input.controls.push(control),
            },
            Event::Resize(..) => input.resized = true,
            _ => {}
        }
    }
    Ok(input)
}
