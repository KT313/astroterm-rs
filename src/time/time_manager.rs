use std::time::Duration;

use astroterm::astro::SimulationClock;
use astroterm::cli::Arguments;
use astroterm::model::Config;
use astroterm::timing::StepTimes;

/// Record startup work before a simulation clock exists.
pub struct StartupTiming {
    pub step_times: StepTimes,
}

/// Fully initialized timing state required by the frame loop.
pub struct TimeManager {
    pub step_times: StepTimes,
    pub min_frame_duration: Duration,
    pub clock: SimulationClock,
}

impl TimeManager {
    /// Finish the previous measurement and start a dotted path; END stops measurement.
    pub fn measure(&mut self, path: &'static str) {
        self.step_times.measure_checkpoint(path);
    }
}

impl StartupTiming {
    /// Begin recording startup work without starting the simulation clock.
    pub fn new(arguments: &Arguments) -> Self {
        // Enable the requested diagnostics before any startup data is loaded.
        let capture_trace = arguments.debug_singleframe || arguments.debug_frames.is_some();
        let mut step_times = StepTimes::with_trace(capture_trace);
        step_times.enable_details(arguments.debug_frametimes);

        Self { step_times }
    }

    /// Consume startup timing and start simulated time after setup and startup reporting finish.
    pub fn start_simulation(self, config: &Config) -> TimeManager {
        // Use the validated frame rate to set the minimum interval between frames.
        let min_frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));

        // Benchmarks advance by frame index; interactive runs advance with elapsed real time.
        let start_date = config.simulation.start_julian_date;
        let clock = match &config.frame_benchmark {
            Some(benchmark) => SimulationClock::start_fixed_step(start_date, benchmark.step_seconds),
            None => SimulationClock::start(start_date, config.simulation.speed),
        };

        // Move the collected measurements into the fully initialized runtime owner.
        TimeManager {
            step_times: self.step_times,
            min_frame_duration,
            clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astroterm::cli::build_config;
    use clap::Parser;

    #[test]
    fn startup_records_timings_without_starting_the_clock() {
        let arguments = Arguments::try_parse_from(["astroterm", "--debug-singleframe"]).unwrap();
        let mut startup = StartupTiming::new(&arguments);
        startup.step_times.measure("Startup fixture", || {});
        assert_eq!(startup.step_times.trace().unwrap().steps[0].name, "Startup fixture");

        let mut config = build_config(arguments, &[]).unwrap();
        config.fps = 60;
        let manager = startup.start_simulation(&config);
        assert_eq!(manager.min_frame_duration, Duration::from_secs_f64(1.0 / 60.0));
        let clock = &manager.clock;
        assert_eq!(clock.start_julian_date(), config.simulation.start_julian_date);
        assert_eq!(clock.speed(), config.simulation.speed);
        assert_eq!(clock.frame_step_seconds(), None);
        assert_eq!(manager.step_times.trace().unwrap().steps.len(), 1);
    }

    #[test]
    fn benchmark_clock_uses_the_configured_frame_step() {
        let arguments = Arguments::try_parse_from([
            "astroterm", "--debug-frames", "3", "--debug-frame-report", "unused.jsonl",
            "--debug-frame-step-seconds", "-2", "-d", "2025-03-01T11:00:00",
        ]).unwrap();
        let startup = StartupTiming::new(&arguments);
        let config = build_config(arguments, &[]).unwrap();
        let mut manager = startup.start_simulation(&config);

        let clock = &mut manager.clock;
        assert_eq!(clock.frame_step_seconds(), Some(-2.0));
        assert_eq!(clock.julian_date(), config.simulation.start_julian_date);
        clock.set_frame_index(2).unwrap();
        assert_eq!(clock.julian_date(), config.simulation.start_julian_date - 4.0 / 86400.0);
    }

    #[test]
    fn manager_checkpoints_reach_the_shared_frame_report() {
        let arguments = Arguments::try_parse_from(["astroterm", "--debug-singleframe"]).unwrap();
        let startup = StartupTiming::new(&arguments);
        let config = build_config(arguments, &[]).unwrap();
        let mut manager = startup.start_simulation(&config);
        manager.step_times.begin_frame();
        manager.measure("Solar system.Earth state");
        manager.measure("END");

        assert_eq!(manager.step_times.steps()[0].name, "Solar system");
        assert_eq!(manager.step_times.steps()[1].name, "Earth state");
        assert!(manager.step_times.capture_steps().is_ok());
    }
}
