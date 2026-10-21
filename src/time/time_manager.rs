use std::{io, time::Duration};

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
    start_time_ms: i64,
}

impl TimeManager {
    /// Return the current simulation date as milliseconds since Gregorian year zero.
    pub fn get_current_simulation_time(&self) -> io::Result<i64> {
        // Single-frame diagnostics use the exact requested date, regardless of time spent preparing the frame.
        if self.debug_singleframe {
            return Ok(self.start_time_ms);
        }

        // Interactive runs and benchmarks use the date currently held by their simulation clock.
        self.clock.get_current_time_ms()
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
    pub fn start_simulation(self, start_time_ms: i64, speed: f64, frames_per_second: u32, frame_step_ms: Option<i64>, debug_singleframe: bool) -> TimeManager {
        // Use the validated frame rate to set the minimum interval between frames.
        let min_frame_duration = Duration::from_secs_f64(1.0 / f64::from(frames_per_second));

        // Benchmarks advance by frame index; interactive runs advance with elapsed real time.
        let clock = match frame_step_ms {
            Some(step_ms) => SimulationClock::start_fixed_step(start_time_ms, step_ms),
            None => SimulationClock::start(start_time_ms, speed),
        };

        // Move the collected measurements into the fully initialized runtime owner.
        TimeManager {
            step_times: self.step_times,
            min_frame_duration,
            clock,
            debug_singleframe,
            start_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START_DATE: i64 = 63_113_947_200_000;

    #[test]
    fn single_frame_time_uses_the_stored_start_date_instead_of_the_current_clock() {
        let startup = StartupTiming::new(true, false);
        let mut manager = startup.start_simulation(START_DATE, 1.0, 60, None, true);
        manager.clock = SimulationClock::start(START_DATE + 1, 0.0);

        assert_eq!(manager.get_current_simulation_time().unwrap(), START_DATE);
    }

    #[test]
    fn normal_time_follows_the_clock_for_paused_forward_and_reverse_frames() {
        for step_ms in [0, 2000, -2000] {
            let startup = StartupTiming::new(true, false);
            let mut manager = startup.start_simulation(START_DATE, 1.0, 60, Some(step_ms), false);
            manager.clock.set_frame_index(3).unwrap();

            assert_eq!(manager.get_current_simulation_time().unwrap(), START_DATE + 3 * step_ms);
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
        assert_eq!(clock.start_time_ms(), START_DATE);
        assert_eq!(clock.speed(), 100.0);
        assert_eq!(clock.frame_step_ms(), None);
        assert_eq!(manager.step_times.trace().unwrap().steps.len(), 1);
    }

    #[test]
    fn benchmark_clock_uses_the_configured_frame_step() {
        let startup = StartupTiming::new(true, false);
        let mut manager = startup.start_simulation(START_DATE, 1.0, 60, Some(-2000), false);

        let clock = &mut manager.clock;
        assert_eq!(clock.frame_step_ms(), Some(-2000));
        assert_eq!(clock.get_current_time_ms().unwrap(), START_DATE);
        clock.set_frame_index(2).unwrap();
        assert_eq!(clock.get_current_time_ms().unwrap(), START_DATE - 4000);
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
