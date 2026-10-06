//! Key bindings: which terminal keys trigger which controls, and the help text listing them. Both come from one table,
//! so they can't drift apart.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::controls::Control;

/// One line of the help text and the keys behind it.
struct KeyBinding {
    keys: &'static str,
    action: &'static str,
    bindings: &'static [(KeyCode, Control)],
}

/// All key bindings, in help text order. Ctrl-C also quits; it is handled separately, as it needs a modifier.
const KEY_BINDINGS: &[KeyBinding] = &[
    KeyBinding {
        keys: "arrows, h j k l",
        action: "Look around (turns the overhead view into a facing view)",
        bindings: &[
            (KeyCode::Left, Control::PanLeft),
            (KeyCode::Char('h'), Control::PanLeft),
            (KeyCode::Right, Control::PanRight),
            (KeyCode::Char('l'), Control::PanRight),
            (KeyCode::Up, Control::PanUp),
            (KeyCode::Char('k'), Control::PanUp),
            (KeyCode::Down, Control::PanDown),
            (KeyCode::Char('j'), Control::PanDown),
        ],
    },
    KeyBinding {
        keys: "+ -",
        action: "Zoom in / out",
        bindings: &[
            (KeyCode::Char('+'), Control::ZoomIn),
            (KeyCode::Char('='), Control::ZoomIn),
            (KeyCode::Char('-'), Control::ZoomOut),
            (KeyCode::Char('_'), Control::ZoomOut),
        ],
    },
    KeyBinding {
        keys: "space",
        action: "Pause / resume time",
        bindings: &[(KeyCode::Char(' '), Control::TogglePause)],
    },
    KeyBinding {
        keys: "] [",
        action: "Speed time up / slow it down (10x)",
        bindings: &[
            (KeyCode::Char(']'), Control::SpeedUp),
            (KeyCode::Char('['), Control::SlowDown),
        ],
    },
    KeyBinding {
        keys: "r",
        action: "Reverse time",
        bindings: &[(KeyCode::Char('r'), Control::ReverseTime)],
    },
    KeyBinding {
        keys: "0",
        action: "Reset the view",
        bindings: &[(KeyCode::Char('0'), Control::ResetView)],
    },
    KeyBinding {
        keys: "q, Esc, Ctrl-C",
        action: "Quit",
        bindings: &[(KeyCode::Char('q'), Control::Quit), (KeyCode::Esc, Control::Quit)],
    },
];

/// Width of the keys column in the help text.
const KEYS_COLUMN_WIDTH: usize = 18;

/// The control bound to a key, if any. With `quit_on_any_key`, every key quits.
///
/// Quit keys work with any modifiers (raw mode turns Ctrl-C into a key press). Other keys don't trigger anything while
/// Ctrl or Alt is held, but Shift is allowed, as some layouts need it for `+` or `]`.
pub fn key_to_control(key: &KeyEvent, quit_on_any_key: bool) -> Option<Control> {
    let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    if quit_on_any_key || ctrl_c {
        return Some(Control::Quit);
    }

    let (_, control) = KEY_BINDINGS
        .iter()
        .flat_map(|binding| binding.bindings)
        .find(|(code, _)| *code == key.code)?;
    let modified = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    (*control == Control::Quit || !modified).then_some(*control)
}

/// The key bindings for the help text, one line per binding.
pub fn format_key_bindings_help() -> String {
    let mut help = String::from("Keys while running:");
    for binding in KEY_BINDINGS {
        help.push_str(&format!("\n  {:<KEYS_COLUMN_WIDTH$}{}", binding.keys, binding.action));
    }
    help
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_map_to_controls() {
        assert_eq!(key_to_control(&key(KeyCode::Left), false), Some(Control::PanLeft));
        assert_eq!(key_to_control(&key(KeyCode::Char('k')), false), Some(Control::PanUp));
        assert_eq!(key_to_control(&key(KeyCode::Char('=')), false), Some(Control::ZoomIn));
        assert_eq!(
            key_to_control(&key(KeyCode::Char(' ')), false),
            Some(Control::TogglePause)
        );
        assert_eq!(key_to_control(&key(KeyCode::Char('x')), false), None);
        let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(key_to_control(&ctrl_l, false), None);
        let shift_plus = KeyEvent::new(KeyCode::Char('+'), KeyModifiers::SHIFT);
        assert_eq!(key_to_control(&shift_plus, false), Some(Control::ZoomIn));
    }

    #[test]
    fn quit_keys() {
        let quits = |code, modifiers, any| key_to_control(&KeyEvent::new(code, modifiers), any) == Some(Control::Quit);
        assert!(quits(KeyCode::Char('q'), KeyModifiers::NONE, false));
        assert!(quits(KeyCode::Esc, KeyModifiers::NONE, false));
        assert!(quits(KeyCode::Char('c'), KeyModifiers::CONTROL, false));
        assert!(quits(KeyCode::Char('q'), KeyModifiers::ALT, false));
        assert!(!quits(KeyCode::Char('c'), KeyModifiers::NONE, false));
        assert!(quits(KeyCode::Char('x'), KeyModifiers::NONE, true));
        assert!(quits(KeyCode::Char('h'), KeyModifiers::NONE, true));
    }

    #[test]
    fn help_lists_every_binding() {
        assert_eq!(
            format_key_bindings_help(),
            "\
Keys while running:
  arrows, h j k l   Look around (turns the overhead view into a facing view)
  + -               Zoom in / out
  space             Pause / resume time
  ] [               Speed time up / slow it down (10x)
  r                 Reverse time
  0                 Reset the view
  q, Esc, Ctrl-C    Quit"
        );
    }
}
