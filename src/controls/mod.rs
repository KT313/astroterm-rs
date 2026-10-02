//! Interactive controls: which keys do what, and their effect on the view and the simulation clock.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::astro::SimulationClock;
use crate::projection::View;

/// Fraction of the field of view that one pan step turns the view.
const PAN_STEP_FRACTION: f64 = 1.0 / 20.0;

/// Field of view change of one zoom step.
const ZOOM_FACTOR: f64 = 1.25;

/// Speed change of one speed step.
const SPEED_FACTOR: f64 = 10.0;

/// Key bindings, for the help text.
pub const KEY_BINDINGS_HELP: &str = "\
Keys while running:
  arrows, h j k l   Look around (turns the overhead view into a facing view)
  + -               Zoom in / out
  space             Pause / resume time
  ] [               Speed time up / slow it down (10x)
  r                 Reverse time
  0                 Reset the view
  q, Esc, Ctrl-C    Quit";

/// An action triggered by a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
    TogglePause,
    SpeedUp,
    SlowDown,
    ReverseTime,
    ResetView,
}

/// The control bound to a key, if any.
pub fn key_to_control(key: &KeyEvent) -> Option<Control> {
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    let control = match key.code {
        KeyCode::Left | KeyCode::Char('h') => Control::PanLeft,
        KeyCode::Right | KeyCode::Char('l') => Control::PanRight,
        KeyCode::Up | KeyCode::Char('k') => Control::PanUp,
        KeyCode::Down | KeyCode::Char('j') => Control::PanDown,
        KeyCode::Char('+' | '=') => Control::ZoomIn,
        KeyCode::Char('-' | '_') => Control::ZoomOut,
        KeyCode::Char(' ') => Control::TogglePause,
        KeyCode::Char(']') => Control::SpeedUp,
        KeyCode::Char('[') => Control::SlowDown,
        KeyCode::Char('r') => Control::ReverseTime,
        KeyCode::Char('0') => Control::ResetView,
        _ => return None,
    };
    Some(control)
}

/// Apply a control to the view or the clock. `initial_view` is the view to reset to.
pub fn apply_control(control: Control, view: &mut View, clock: &mut SimulationClock, initial_view: &View) {
    let pan_step = (view.fov_degrees * PAN_STEP_FRACTION).to_radians();
    match control {
        Control::PanLeft => view.pan(-pan_step, 0.0),
        Control::PanRight => view.pan(pan_step, 0.0),
        Control::PanUp => view.pan(0.0, pan_step),
        Control::PanDown => view.pan(0.0, -pan_step),
        Control::ZoomIn => view.zoom(ZOOM_FACTOR),
        Control::ZoomOut => view.zoom(1.0 / ZOOM_FACTOR),
        Control::TogglePause => clock.toggle_pause(),
        Control::SpeedUp => clock.set_speed(clock.speed() * SPEED_FACTOR),
        Control::SlowDown => clock.set_speed(clock.speed() / SPEED_FACTOR),
        Control::ReverseTime => clock.set_speed(-clock.speed()),
        Control::ResetView => *view = *initial_view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::J2000;
    use crate::projection::ViewCenter;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_map_to_controls() {
        assert_eq!(key_to_control(&key(KeyCode::Left)), Some(Control::PanLeft));
        assert_eq!(key_to_control(&key(KeyCode::Char('k'))), Some(Control::PanUp));
        assert_eq!(key_to_control(&key(KeyCode::Char('='))), Some(Control::ZoomIn));
        assert_eq!(key_to_control(&key(KeyCode::Char(' '))), Some(Control::TogglePause));
        assert_eq!(key_to_control(&key(KeyCode::Char('x'))), None);
        assert_eq!(
            key_to_control(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn controls_change_view_and_clock() {
        let initial_view = View::default();
        let (mut view, mut clock) = (initial_view, SimulationClock::start(J2000, 1.0));

        apply_control(Control::ZoomIn, &mut view, &mut clock, &initial_view);
        assert_eq!(view.fov_degrees, 144.0);
        apply_control(Control::PanRight, &mut view, &mut clock, &initial_view);
        let ViewCenter::Facing { azimuth, .. } = view.center else {
            panic!("facing view expected")
        };
        assert!((azimuth.to_degrees() - (180.0 + 144.0 / 20.0)).abs() < 1e-9); // steps scale with the zoom

        apply_control(Control::SpeedUp, &mut view, &mut clock, &initial_view);
        apply_control(Control::ReverseTime, &mut view, &mut clock, &initial_view);
        assert_eq!(clock.speed(), -10.0);
        apply_control(Control::TogglePause, &mut view, &mut clock, &initial_view);
        assert!(clock.is_paused());

        apply_control(Control::ResetView, &mut view, &mut clock, &initial_view);
        assert_eq!(view, initial_view);
    }
}
