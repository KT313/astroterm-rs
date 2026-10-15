//! Integer simulation timestamps with a private fractional phase for smooth clock-rate changes.
use std::{io, time::Instant};

#[derive(Clone, Copy, Debug)]
pub struct SimulationClock {
    start_time_ms: i64,
    anchor_time_ms: i64,
    anchor_instant: Instant,
    fractional_ms: f64,
    speed: f64,
    paused: bool,
    frame_step_ms: Option<i64>,
}

fn overflow() -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, "simulation time exceeds the i64 millisecond range") }

impl SimulationClock {
    /// Start at a timestamp in milliseconds since Gregorian year zero. Speed must already be finite.
    pub fn start(start_time_ms: i64, speed: f64) -> Self {
        assert!(speed.is_finite(), "simulation speed must be finite");
        Self { start_time_ms, anchor_time_ms: start_time_ms, anchor_instant: Instant::now(), fractional_ms: 0.0,
            speed, paused: false, frame_step_ms: None }
    }

    /// Benchmarks advance from the original timestamp by an exact integer step, ignoring wall time.
    pub fn start_fixed_step(start_time_ms: i64, step_ms: i64) -> Self {
        Self { frame_step_ms: Some(step_ms), paused: step_ms == 0, ..Self::start(start_time_ms, 0.0) }
    }

    pub fn frame_step_ms(&self) -> Option<i64> { self.frame_step_ms }
    pub fn start_time_ms(&self) -> i64 { self.start_time_ms }
    pub fn speed(&self) -> f64 { self.speed }
    pub fn is_paused(&self) -> bool { self.paused }

    /// Calculate from the original timestamp, without accumulating rounding or mutating state on failure.
    pub fn set_frame_index(&mut self, index: u32) -> io::Result<()> {
        if let Some(step_ms) = self.frame_step_ms {
            let next = i128::from(self.start_time_ms) + i128::from(index) * i128::from(step_ms);
            self.anchor_time_ms = i64::try_from(next).map_err(|_| overflow())?;
        }
        Ok(())
    }

    /// Read the current UTC/UT timestamp, flooring fractional milliseconds without discarding their phase.
    pub fn get_current_time_ms(&self) -> io::Result<i64> {
        self.read_at(Instant::now()).map(|(time_ms, _)| time_ms)
    }

    pub fn set_speed(&mut self, speed: f64) -> io::Result<()> { self.set_speed_at(speed, Instant::now()) }
    pub fn toggle_pause(&mut self) -> io::Result<()> { self.toggle_pause_at(Instant::now()) }

    fn read_at(&self, now: Instant) -> io::Result<(i64, f64)> {
        // Frozen clocks retain their sub-millisecond phase; reading never updates the anchor.
        if self.paused || self.frame_step_ms.is_some() { return Ok((self.anchor_time_ms, self.fractional_ms)); }
        let elapsed = now.checked_duration_since(self.anchor_instant)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "clock instant precedes its anchor"))?;
        let advance_ms = elapsed.as_secs_f64() * 1000.0 * self.speed + self.fractional_ms;
        let whole_ms = advance_ms.floor();

        // Check before casting: float-to-integer casts would otherwise silently saturate.
        let integer_limit = 2.0_f64.powi(127);
        if !whole_ms.is_finite() || whole_ms < -integer_limit || whole_ms >= integer_limit { return Err(overflow()); }
        let next = i128::from(self.anchor_time_ms).checked_add(whole_ms as i128).ok_or_else(overflow)?;
        let next = i64::try_from(next).map_err(|_| overflow())?;
        let fraction = (advance_ms - whole_ms).min(1.0_f64.next_down()); // subtraction can round a tiny negative phase up to 1
        Ok((next, fraction))
    }

    fn anchor_at(&mut self, now: Instant) -> io::Result<()> {
        // Calculate everything before committing the new anchor so an error leaves the clock intact.
        let (time_ms, fractional_ms) = self.read_at(now)?;
        self.anchor_time_ms = time_ms;
        self.fractional_ms = fractional_ms;
        self.anchor_instant = now;
        Ok(())
    }

    fn set_speed_at(&mut self, speed: f64, now: Instant) -> io::Result<()> {
        if !speed.is_finite() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "simulation speed must be finite")); }
        self.anchor_at(now)?;
        self.speed = speed;
        Ok(())
    }

    fn toggle_pause_at(&mut self, now: Instant) -> io::Result<()> {
        self.anchor_at(now)?;
        self.paused = !self.paused;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slow_motion_preserves_fractional_progress_across_reanchors() {
        let mut clock = SimulationClock::start(0, 0.125);
        let origin = clock.anchor_instant;
        for index in 1..=8 { clock.set_speed_at(0.125, origin + Duration::from_millis(index)).unwrap(); }
        assert_eq!(clock.anchor_time_ms, 1);
        assert_eq!(clock.fractional_ms, 0.0);
        let clock = SimulationClock::start(0, 0.1);
        for index in 0..100 { clock.read_at(clock.anchor_instant + Duration::from_millis(index)).unwrap(); }
        assert_eq!(clock.read_at(clock.anchor_instant + Duration::from_millis(100)).unwrap().0, 10);
    }

    #[test]
    fn pause_speed_change_and_reverse_preserve_phase() {
        let mut clock = SimulationClock::start(0, -0.25);
        let origin = clock.anchor_instant;
        clock.toggle_pause_at(origin + Duration::from_millis(1)).unwrap();
        assert_eq!((clock.anchor_time_ms, clock.fractional_ms), (-1, 0.75));
        clock.set_speed_at(0.25, origin + Duration::from_secs(5)).unwrap();
        assert_eq!(clock.read_at(origin + Duration::from_secs(10)).unwrap(), (-1, 0.75));
        clock.toggle_pause_at(origin + Duration::from_secs(10)).unwrap();
        assert_eq!(clock.read_at(origin + Duration::from_secs(10) + Duration::from_millis(1)).unwrap(), (0, 0.0));
    }

    #[test]
    fn extreme_anchors_stay_exact_and_overflow_does_not_mutate_state() {
        for (start, speed) in [(i64::MAX - 1, 1.0), (i64::MIN + 1, -1.0)] {
            let mut clock = SimulationClock::start(start, speed);
            let origin = clock.anchor_instant;
            assert_eq!(clock.read_at(origin + Duration::from_millis(1)).unwrap().0, if speed > 0.0 { i64::MAX } else { i64::MIN });
            assert!(clock.toggle_pause_at(origin + Duration::from_millis(2)).is_err());
            assert_eq!(clock.anchor_time_ms, start);
            assert_eq!(clock.anchor_instant, origin);
            assert!(!clock.is_paused());
            assert!(clock.set_speed_at(f64::NAN, origin).is_err());
            assert_eq!(clock.speed(), speed);
        }
    }

    #[test]
    fn benchmark_schedules_are_exact_and_overflow_is_transactional() {
        for step in [0, 1, -1, 1000, -2500] {
            let mut clock = SimulationClock::start_fixed_step(-100, step);
            for index in [0, 1, 100_000, u32::MAX] {
                clock.set_frame_index(index).unwrap();
                assert_eq!(clock.get_current_time_ms().unwrap(), -100 + i64::from(index) * step);
            }
        }
        let mut clock = SimulationClock::start_fixed_step(i64::MAX, 1);
        assert!(clock.set_frame_index(1).is_err());
        assert_eq!(clock.get_current_time_ms().unwrap(), i64::MAX);
    }
}
