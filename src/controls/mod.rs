//! Interactive controls and their effect on the view and the simulation clock. Which keys or other input trigger
//! them is up to the backend (see `terminal::keys`).

use crate::constants::{MAX_INTERACTIVE_SPEED, PAN_STEP_FRACTION, SPEED_FACTOR, ZOOM_FACTOR};
use crate::astro::SimulationClock;
use crate::model::View;


/// An action the user can trigger while the sky is shown.
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
    Quit,
}

/// Apply a control to the view or the clock. `initial_view` is the view to reset to.
pub fn apply_control(control: Control, view: &mut View, clock: &mut SimulationClock, initial_view: &View) {
    let pan_step = (view.fov_degrees * PAN_STEP_FRACTION).to_radians();
    match control {
        Control::PanLeft => crate::projection::pan_view(view, -pan_step, 0.0),
        Control::PanRight => crate::projection::pan_view(view, pan_step, 0.0),
        Control::PanUp => crate::projection::pan_view(view, 0.0, pan_step),
        Control::PanDown => crate::projection::pan_view(view, 0.0, -pan_step),
        Control::ZoomIn => crate::projection::zoom_view(view, ZOOM_FACTOR),
        Control::ZoomOut => crate::projection::zoom_view(view, 1.0 / ZOOM_FACTOR),
        Control::TogglePause => clock.toggle_pause(),
        Control::SpeedUp => {
            clock.set_speed((clock.speed() * SPEED_FACTOR).clamp(-MAX_INTERACTIVE_SPEED, MAX_INTERACTIVE_SPEED))
        }
        Control::SlowDown => {
            clock.set_speed((clock.speed() / SPEED_FACTOR).clamp(-MAX_INTERACTIVE_SPEED, MAX_INTERACTIVE_SPEED))
        }
        Control::ReverseTime => clock.set_speed(-clock.speed()),
        Control::ResetView => *view = *initial_view,
        Control::Quit => {} // handled by the frame loop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::J2000;
    use crate::model::ViewCenter;

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

    #[test]
    fn repeated_speed_changes_saturate_and_preserve_pause_and_direction() {
        for speed in [1.0, -1.0, f64::MAX, -f64::MAX, 0.0] {
            let initial_view = View::default();
            let mut view = initial_view;
            let mut clock = SimulationClock::start(J2000, 0.0);
            clock.toggle_pause();
            clock.set_speed(speed);
            for _ in 0..400 {
                apply_control(Control::SpeedUp, &mut view, &mut clock, &initial_view);
                assert!(clock.speed().is_finite() && clock.speed().abs() <= MAX_INTERACTIVE_SPEED);
            }
            assert_eq!(
                clock.speed(),
                if speed == 0.0 {
                    0.0
                } else {
                    speed.signum() * MAX_INTERACTIVE_SPEED
                }
            );
            apply_control(Control::SlowDown, &mut view, &mut clock, &initial_view);
            assert!(clock.speed().abs() <= MAX_INTERACTIVE_SPEED / SPEED_FACTOR);
            assert!(clock.is_paused());
            assert_eq!(clock.julian_date(), J2000);
        }
    }
}
