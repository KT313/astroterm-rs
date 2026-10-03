//! Measuring how long the steps of each frame take, smoothed over frames, to find what needs optimizing.

mod trace;
pub use trace::{PipelineTrace, TraceStep};

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
    records: Vec<StepRecord>,
    parents: Vec<&'static str>,
    per_frame: bool,
    trace: Option<PipelineTrace>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct StepRecord {
    parents: Vec<&'static str>,
    initialized: bool,
    previous_average: Option<f64>,
    frame_seconds: Option<f64>,
    frame_calls: usize,
}

impl StepTimes {
    /// Start a frame. Repeated calls within one scope contribute to one frame total before smoothing.
    /// Callers without a frame loop retain the original per-call averaging behavior.
    pub fn begin_frame(&mut self) {
        for (step, record) in self.steps.iter_mut().zip(&mut self.records) {
            if self.per_frame && record.frame_seconds.is_none() && record.initialized {
                step.average_seconds *= EMA_FACTOR; // a skipped optional stage cost zero in the completed frame
            }
            record.previous_average = record.initialized.then_some(step.average_seconds);
            record.frame_seconds = None;
            record.frame_calls = 0;
        }
        self.per_frame = true;
    }

    /// Run a step, add its duration to the step's average, and return its result.
    pub fn measure<T>(&mut self, name: &'static str, run: impl FnOnce() -> T) -> T {
        let trace_index = self.start_trace(name);
        let start = Instant::now();
        let result = run();
        let seconds = start.elapsed().as_secs_f64();
        self.finish_trace(trace_index, seconds);
        self.record(name, seconds);
        result
    }

    /// Measure a stage that also records its own sub-steps.
    pub fn measure_steps<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        let trace_index = self.start_trace(name);
        self.register_step(name); // reserve the parent before its children, so the panel reads in pipeline order
        let start = Instant::now();
        self.parents.push(name);
        self.push_trace_scope(trace_index);
        let result = run(self);
        self.pop_trace_scope(trace_index);
        self.parents.pop();
        let seconds = start.elapsed().as_secs_f64();
        self.finish_trace(trace_index, seconds);
        self.record(name, seconds);
        result
    }

    /// Time bounded-batch passes without a clock per object or thousands of trace entries. Child rows are
    /// explicitly aggregated in first-occurrence order; their calls interleave once per batch.
    pub fn measure_batches<T>(&mut self, name: &'static str, run: impl FnOnce(&mut Self) -> T) -> T {
        self.measure_steps(name, |times| {
            let mut batches = Self::default();
            batches.begin_frame();
            let result = run(&mut batches);
            for (step, record) in batches.steps.iter().zip(&batches.records) {
                assert_eq!(step.depth, 0, "batch passes must be flat");
                let seconds = record.frame_seconds.unwrap_or(0.0);
                let index = times.start_trace(step.name);
                times.finish_trace(index, seconds);
                times.record(step.name, seconds);
                times.describe(step.name, || {
                    format!(
                        "sum of {} batch calls; passes interleave per batch; no per-star timers",
                        record.frame_calls
                    )
                });
            }
            result
        })
    }

    /// The steps measured so far, with their averages.
    pub fn steps(&self) -> &[StepTime] {
        &self.steps
    }

    /// Add a duration to a step's average. A new step starts at its first duration rather than at zero.
    fn record(&mut self, name: &'static str, seconds: f64) {
        let index = self.register_step(name);
        let step = &mut self.steps[index];
        let record = &mut self.records[index];
        record.frame_calls += 1;
        let (previous, sample) = if self.per_frame {
            let total = record.frame_seconds.get_or_insert(0.0);
            *total += seconds;
            (record.previous_average, *total)
        } else {
            (record.initialized.then_some(step.average_seconds), seconds)
        };
        step.average_seconds = previous.map_or(sample, |old| old * EMA_FACTOR + sample * (1.0 - EMA_FACTOR));
        record.initialized = true;
    }

    fn register_step(&mut self, name: &'static str) -> usize {
        if let Some(index) = self
            .steps
            .iter()
            .zip(&self.records)
            .position(|(step, record)| step.name == name && record.parents == self.parents)
        {
            return index;
        }
        self.steps.push(StepTime {
            name,
            depth: self.parents.len(),
            average_seconds: 0.0,
        });
        self.records.push(StepRecord {
            parents: self.parents.clone(),
            ..StepRecord::default()
        });
        self.steps.len() - 1
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

    #[test]
    fn repeated_calls_are_summed_then_smoothed_once_per_frame() {
        let mut times = StepTimes::default();
        times.begin_frame();
        times.record("Samples", 1.0);
        times.record("Samples", 2.0);
        assert_eq!(times.steps()[0].average_seconds, 3.0);
        times.begin_frame();
        times.record("Samples", 4.0);
        times.record("Samples", 5.0);
        assert!((times.steps()[0].average_seconds - (3.0 * 0.95 + 9.0 * 0.05)).abs() < 1e-12);
    }

    #[test]
    fn identical_names_in_distinct_parent_scopes_stay_separate() {
        let mut times = StepTimes::default();
        times.begin_frame();
        times.measure_steps("Simulation", |times| times.record("Samples", 1.0));
        times.measure_steps("Light time", |times| {
            times.record("Samples", 2.0);
            times.record("Samples", 3.0);
        });
        let steps = times.steps();
        assert_eq!(
            steps.iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(),
            [("Simulation", 0), ("Samples", 1), ("Light time", 0), ("Samples", 1),]
        );
        assert_eq!(steps[1].average_seconds, 1.0);
        assert_eq!(steps[3].average_seconds, 5.0);
    }

    #[test]
    fn skipped_optional_steps_decay_towards_zero() {
        let mut times = StepTimes::default();
        times.begin_frame();
        times.record("Refraction", 1.0);
        times.begin_frame();
        times.begin_frame();
        assert_eq!(times.steps()[0].average_seconds, EMA_FACTOR);
    }
}
