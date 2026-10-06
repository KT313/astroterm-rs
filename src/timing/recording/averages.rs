//! Per-step registration and smoothing; frame orchestration stays in pipeline.rs.
use super::{StepRecord, StepTime, StepTimes, EMA_FACTOR};
#[cfg(feature = "memory-diagnostics")]
use super::{memory, run};

impl StepTimes {
    pub(super) fn reset_frame_samples(&mut self) {
        for (step, record) in self.steps.iter_mut().zip(&mut self.records) {
            if self.per_frame && record.frame_seconds.is_none() && record.initialized {
                step.average_seconds *= EMA_FACTOR; // a skipped optional stage cost zero in the completed frame
            }
            record.previous_average = record.initialized.then_some(step.average_seconds);
            record.frame_seconds = None;
            record.frame_calls = 0;
        }
    }

    /// Add a duration to a step's average. A new step starts at its first duration rather than at zero.
    pub(super) fn record(&mut self, name: &'static str, seconds: f64) {
        let Some(index) = self.register_step(name) else {
            #[cfg(feature = "memory-diagnostics")]
            if self.memory_batch { self.memory_completed = None; }
            return;
        };
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_batch { self.memory_completed = Some(memory::MemoryStepId(memory::Target::Batch(index), self.memory_epoch)); }
        let step = &mut self.steps[index];
        let record = &mut self.records[index];
        record.frame_calls = record.frame_calls.saturating_add(1);
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

    pub(super) fn register_step(&mut self, name: &'static str) -> Option<usize> {
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && self.memory_suppressed != 0 {
            self.registry_omitted = self.registry_omitted.saturating_add(1);
            return None;
        }
        if let Some(index) = self
            .steps
            .iter()
            .zip(&self.records)
            .position(|(step, record)| step.name == name && record.parents == self.parents)
        {
            return Some(index);
        }
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && self.steps.len() >= run::MAX_TIMING_PATHS {
            self.registry_omitted = self.registry_omitted.saturating_add(1);
            return None;
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
        Some(self.steps.len() - 1)
    }
}


#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(StepTime);
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StepRecord { parents, memory_events });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StepTimes { steps, records, parents, trace, memory_run });

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
