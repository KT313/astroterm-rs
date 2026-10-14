//! Per-step registration and smoothing; frame orchestration stays in pipeline.rs.
use super::TIMING_EMA_FACTOR;
use super::{StepRecord, StepTime, StepTimes};

impl StepTimes {
    pub(super) fn reset_frame_samples(&mut self) {
        for (step, record) in self.steps.iter_mut().zip(&mut self.records) {
            if self.per_frame && record.frame_seconds.is_none() && record.initialized {
                step.average_seconds *= TIMING_EMA_FACTOR; // a skipped optional stage cost zero in the completed frame
            }
            record.previous_average = record.initialized.then_some(step.average_seconds);
            record.frame_seconds = None;
        }
    }

    /// Add a duration to a step's average. A new step starts at its first duration rather than at zero.
    pub(super) fn record(&mut self, name: &'static str, parents: &[&'static str], seconds: f64) {
        let index = self.register_step(name, parents);
        let step = &mut self.steps[index];
        let record = &mut self.records[index];
        let total = record.frame_seconds.get_or_insert(0.0);
        *total += seconds;
        let previous = if self.per_frame { record.previous_average } else { None };
        step.average_seconds = previous.map_or(*total, |old| old * TIMING_EMA_FACTOR + *total * (1.0 - TIMING_EMA_FACTOR));
        record.initialized = true;
    }

    fn register_step(&mut self, name: &'static str, parents: &[&'static str]) -> usize {
        if let Some(index) = self
            .steps
            .iter()
            .zip(&self.records)
            .position(|(step, record)| step.name == name && record.parents == parents)
        {
            return index;
        }
        self.steps.push(StepTime {
            name,
            depth: parents.len(),
            average_seconds: 0.0,
        });
        self.records.push(StepRecord {
            parents: parents.to_vec(),
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
        times.begin_frame();
        times.record("Stars", &[], 1.0);
        assert_eq!(times.steps()[0].average_seconds, 1.0);
        times.begin_frame();
        times.record("Stars", &[], 3.0);
        assert!((times.steps()[0].average_seconds - (0.95 + 0.15)).abs() < 1e-12);
    }

    #[test]
    fn steps_keep_the_order_they_first_ran_in() {
        let mut times = StepTimes::default();
        for name in ["Stars", "Moon", "Stars", "Draw", "Moon"] {
            times.record(name, &[], 0.001);
        }
        let names: Vec<_> = times.steps().iter().map(|step| step.name).collect();
        assert_eq!(names, ["Stars", "Moon", "Draw"]);
    }

    #[test]
    fn repeated_calls_are_summed_then_smoothed_once_per_frame() {
        let mut times = StepTimes::default();
        times.begin_frame();
        times.record("Samples", &[], 1.0);
        times.record("Samples", &[], 2.0);
        assert_eq!(times.steps()[0].average_seconds, 3.0);
        times.begin_frame();
        times.record("Samples", &[], 4.0);
        times.record("Samples", &[], 5.0);
        assert!((times.steps()[0].average_seconds - (3.0 * 0.95 + 9.0 * 0.05)).abs() < 1e-12);
    }

    #[test]
    fn identical_names_in_distinct_paths_stay_separate() {
        let mut times = StepTimes::default();
        times.begin_frame();
        times.record("Simulation", &[], 1.0);
        times.record("Samples", &["Simulation"], 1.0);
        times.record("Light time", &[], 5.0);
        times.record("Samples", &["Light time"], 2.0);
        times.record("Samples", &["Light time"], 3.0);
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
        times.record("Refraction", &[], 1.0);
        times.begin_frame();
        times.begin_frame();
        assert_eq!(times.steps()[0].average_seconds, TIMING_EMA_FACTOR);
    }
}
