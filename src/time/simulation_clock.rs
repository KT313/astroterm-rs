//! Advance a supplied Julian date using elapsed real time or explicit frame indices.
use std::time::Instant;

const SECONDS_PER_DAY: f64 = 86400.0;

/// Simulation time that runs `speed` times faster than the wall clock (backwards for negative speeds), starting at a
/// given Julian date. It can be paused and its speed changed while running.
///
/// The time is derived from the elapsed wall-clock time since the last change, so slow frames don't make it drift.
#[derive(Clone, Copy, Debug)]
pub struct SimulationClock {
    start_julian_date: f64,
    /// Simulation time at `anchor_instant`; the clock runs on from there.
    anchor_julian_date: f64,
    anchor_instant: Instant,
    speed: f64,
    paused: bool,
    frame_step_seconds: Option<f64>,
}

impl SimulationClock {
    /// A running clock showing the UTC/UT date `start_julian_date_utc` now.
    pub fn start(start_julian_date_utc: f64, speed: f64) -> SimulationClock {
        SimulationClock {
            start_julian_date: start_julian_date_utc,
            anchor_julian_date: start_julian_date_utc,
            anchor_instant: Instant::now(),
            speed,
            paused: false,
            frame_step_seconds: None,
        }
    }

    /// A clock advanced explicitly by frame index; wall time cannot change its simulated date.
    pub fn start_fixed_step(start: f64, step_seconds: f64) -> Self {
        Self { frame_step_seconds: Some(step_seconds), paused: step_seconds == 0.0, ..Self::start(start, 0.0) }
    }

    pub fn frame_step_seconds(&self) -> Option<f64> { self.frame_step_seconds }

    /// Set a zero-based frame index directly from the original date, avoiding accumulated rounding drift.
    pub fn set_frame_index(&mut self, index: u32) -> std::io::Result<()> {
        if let Some(step) = self.frame_step_seconds {
            let date = self.start_julian_date + (f64::from(index) * step) / SECONDS_PER_DAY;
            if !date.is_finite() { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "benchmark frame date is not finite")); }
            self.anchor_julian_date = date;
        }
        Ok(())
    }

    /// The current simulation time.
    pub fn julian_date(&self) -> f64 {
        if self.paused || self.frame_step_seconds.is_some() {
            return self.anchor_julian_date;
        }
        self.anchor_julian_date + self.anchor_instant.elapsed().as_secs_f64() / SECONDS_PER_DAY * self.speed
    }

    /// The simulation time the clock started at.
    pub fn start_julian_date(&self) -> f64 {
        self.start_julian_date
    }

    /// Simulated days per real day.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Change the speed from now on.
    pub fn set_speed(&mut self, speed: f64) {
        self.anchor_to_now();
        self.speed = speed;
    }

    /// Stop or resume the clock.
    pub fn toggle_pause(&mut self) {
        self.anchor_to_now();
        self.paused = !self.paused;
    }

    /// Continue from the current simulation time, so later changes don't affect the time that has already passed.
    fn anchor_to_now(&mut self) {
        self.anchor_julian_date = self.julian_date();
        self.anchor_instant = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const J2000: f64 = 2451545.0;

    #[test]
    fn paused_clock_stands_still_and_resumes_where_it_stopped() {
        let mut clock = SimulationClock::start(J2000, 1e6);
        clock.toggle_pause();
        let paused_at = clock.julian_date();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(clock.julian_date(), paused_at);
        assert!(clock.is_paused());

        clock.set_speed(-1e6); // changing the speed keeps the clock paused
        assert_eq!(clock.julian_date(), paused_at);
        clock.toggle_pause();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(clock.julian_date() < paused_at); // running backwards
        assert_eq!((clock.speed(), clock.start_julian_date()), (-1e6, J2000));
    }

    #[test]
    fn simulation_clock_starts_at_the_start_date() {
        let clock = SimulationClock::start(J2000, 1000.0);
        let elapsed = clock.julian_date() - J2000;
        assert!((0.0..0.01).contains(&elapsed));
    }
}

#[cfg(test)]
mod fixed_frame_tests {
    use super::*;
    const J2000: f64 = 2451545.0;
    #[test]
    fn fixed_steps_use_the_original_epoch_and_do_not_read_wall_time() {
        for step in [0.0, 1.0, -2.5] {
            let mut clock = SimulationClock::start_fixed_step(J2000, step);
            for index in [0, 1, 100_000, u32::MAX] {
                clock.set_frame_index(index).unwrap();
                let expected = J2000 + (f64::from(index) * step) / SECONDS_PER_DAY;
                assert_eq!(clock.julian_date(), expected);
                assert_eq!(clock.julian_date(), expected);
                assert_eq!(clock.start_julian_date(), J2000);
            }
        }
        assert!(SimulationClock::start_fixed_step(J2000, f64::MAX).set_frame_index(2).is_err());
    }
}
