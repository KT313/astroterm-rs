use std::time::Duration;

use super::{SimulationClock, StepTimes};

/// Record startup work before a simulation clock exists.
pub struct StartupTiming {
    pub step_times: StepTimes,
}

/// Fully initialized timing state required by the frame loop.
pub struct TimeManager {
    pub step_times: StepTimes,
    pub min_frame_duration: Duration,
    pub clock: SimulationClock,
    debug_singleframe: bool,
    start_julian_date: f64,
}

impl TimeManager {
    /// Return the current simulation date as a UTC Julian date.
    pub fn get_current_simulation_time(&self) -> f64 {
        // Single-frame diagnostics use the exact requested date, regardless of time spent preparing the frame.
        if self.debug_singleframe {
            return self.start_julian_date;
        }

        // Interactive runs and benchmarks use the date currently held by their simulation clock.
        self.clock.julian_date()
    }

    /// Finish the previous measurement and start a dotted path; END stops measurement.
    pub fn measure(&mut self, path: &'static str) {
        self.step_times.measure(path);
    }
}

impl StartupTiming {
    /// Begin recording startup work without starting the simulation clock.
    pub fn new(capture_trace: bool, show_details: bool) -> Self {
        // Enable the requested diagnostics before any startup data is loaded.
        let mut step_times = StepTimes::with_trace(capture_trace);
        step_times.enable_details(show_details);

        Self { step_times }
    }

    /// Consume startup timing and start simulated time after setup and startup reporting finish.
    pub fn start_simulation(self, start_date: f64, speed: f64, frames_per_second: u32, frame_step_seconds: Option<f64>, debug_singleframe: bool) -> TimeManager {
        // Use the validated frame rate to set the minimum interval between frames.
        let min_frame_duration = Duration::from_secs_f64(1.0 / f64::from(frames_per_second));

        // Benchmarks advance by frame index; interactive runs advance with elapsed real time.
        let clock = match frame_step_seconds {
            Some(step_seconds) => SimulationClock::start_fixed_step(start_date, step_seconds),
            None => SimulationClock::start(start_date, speed),
        };

        // Move the collected measurements into the fully initialized runtime owner.
        TimeManager {
            step_times: self.step_times,
            min_frame_duration,
            clock,
            debug_singleframe,
            start_julian_date: start_date,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START_DATE: f64 = 2451545.0;

    #[test]
    fn single_frame_time_uses_the_stored_start_date_instead_of_the_current_clock() {
        let startup = StartupTiming::new(true, false);
        let mut manager = startup.start_simulation(START_DATE, 1.0, 60, None, true);
        manager.clock = SimulationClock::start(START_DATE + 1.0, 0.0);

        assert_eq!(manager.get_current_simulation_time(), START_DATE);
    }

    #[test]
    fn normal_time_follows_the_clock_for_paused_forward_and_reverse_frames() {
        for step_seconds in [0.0, 2.0, -2.0] {
            let startup = StartupTiming::new(true, false);
            let mut manager = startup.start_simulation(START_DATE, 1.0, 60, Some(step_seconds), false);
            manager.clock.set_frame_index(3).unwrap();

            assert_eq!(manager.get_current_simulation_time(), START_DATE + 3.0 * step_seconds / 86400.0);
        }
    }

    #[test]
    fn startup_records_timings_without_starting_the_clock() {
        let mut startup = StartupTiming::new(true, false);
        startup.step_times.measure("Startup fixture");
        startup.step_times.measure("END");
        assert_eq!(startup.step_times.trace().unwrap().steps[0].name, "Startup fixture");

        let manager = startup.start_simulation(START_DATE, 100.0, 60, None, false);
        assert_eq!(manager.min_frame_duration, Duration::from_secs_f64(1.0 / 60.0));
        let clock = &manager.clock;
        assert_eq!(clock.start_julian_date(), START_DATE);
        assert_eq!(clock.speed(), 100.0);
        assert_eq!(clock.frame_step_seconds(), None);
        assert_eq!(manager.step_times.trace().unwrap().steps.len(), 1);
    }

    #[test]
    fn benchmark_clock_uses_the_configured_frame_step() {
        let startup = StartupTiming::new(true, false);
        let mut manager = startup.start_simulation(START_DATE, 1.0, 60, Some(-2.0), false);

        let clock = &mut manager.clock;
        assert_eq!(clock.frame_step_seconds(), Some(-2.0));
        assert_eq!(clock.julian_date(), START_DATE);
        clock.set_frame_index(2).unwrap();
        assert_eq!(clock.julian_date(), START_DATE - 4.0 / 86400.0);
    }

    #[test]
    fn manager_checkpoints_reach_the_shared_frame_report() {
        let startup = StartupTiming::new(true, false);
        let mut manager = startup.start_simulation(START_DATE, 1.0, 60, None, false);
        manager.step_times.begin_frame();
        manager.measure("Solar system.Earth state");
        manager.measure("END");

        assert_eq!(manager.step_times.steps()[0].name, "Solar system");
        assert_eq!(manager.step_times.steps()[1].name, "Earth state");
        assert!(manager.step_times.capture_steps().is_ok());
    }
}
