//! Measuring how long the steps of each frame take, smoothed over frames, to find what needs optimizing.

use std::time::Instant;

/// Weight of the previous average in the exponential moving average; the newest frame gets the rest.
const EMA_FACTOR: f64 = 0.95;

/// The smoothed duration of one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepTime {
    pub name: &'static str,
    pub depth: usize,
    pub average_seconds: f64,
}

/// Smoothed durations of named steps, one exponential moving average per step, in the order the steps first ran.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepTimes {
    steps: Vec<StepTime>,
    depth: usize,
}

impl StepTimes {
    /// Run a step, add its duration to the step's average, and return its result.
    pub fn measure<T>(&mut self, name: &'static str, run: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let result = run();
        self.record(name, start.elapsed().as_secs_f64());
        result
    }

    /// Measure a stage that also records its own sub-steps.
    pub fn measure_steps<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        let start = Instant::now();
        self.depth += 1;
        let result = run(self);
        self.depth -= 1;
        self.record(name, start.elapsed().as_secs_f64());
        result
    }

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }

    /// Add a duration to a step's average. A new step starts at its first duration rather than at zero.
    fn record(&mut self, name: &'static str, seconds: f64) {
        match self.steps.iter_mut().find(|step| step.name == name) {
            Some(step) => step.average_seconds = step.average_seconds * EMA_FACTOR + seconds * (1.0 - EMA_FACTOR),
            None => self.steps.push(StepTime {
                name,
                depth: self.depth,
                average_seconds: seconds,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_start_at_the_first_duration_and_then_move_slowly() {
        let mut times = StepTimes::default();
        times.record("Stars", 1.0);
        assert_eq!(times.steps()[0].average_seconds, 1.0);
        times.record("Stars", 3.0);
        assert!((times.steps()[0].average_seconds - (0.95 + 0.15)).abs() < 1e-12);
    }

    #[test]
    fn steps_keep_the_order_they_first_ran_in() {
        let mut times = StepTimes::default();
        for name in ["Stars", "Moon", "Stars", "Draw", "Moon"] {
            times.record(name, 0.001);
        }
        let names: Vec<_> = times.steps().iter().map(|step| step.name).collect();
        assert_eq!(names, ["Stars", "Moon", "Draw"]);
    }

    #[test]
    fn measure_returns_the_result_and_records_the_step() {
        let mut times = StepTimes::default();
        assert_eq!(times.measure("Answer", || 42), 42);
        assert_eq!(times.steps().len(), 1);
        assert!(times.steps()[0].average_seconds >= 0.0);
    }
}
