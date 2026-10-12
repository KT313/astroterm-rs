//! Aggregate flat batch passes while preserving their existing event and timing boundaries.
use super::StepTimes;

impl StepTimes {
    pub(super) fn prepare_batch_times(&self) -> Self {
        let mut batches = Self::default();
        batches.begin_frame();
        batches
    }

    pub(super) fn record_batch_results(&mut self, batches: &Self) {
        for (step, record) in batches.steps.iter().zip(&batches.records) {
            assert_eq!(step.depth, 0, "batch passes must be flat");
            let seconds = record.frame_seconds.unwrap_or(0.0);
            let index = self.start_trace(step.name);
            self.finish_trace(index, seconds);
            self.record(step.name, seconds);
            self.describe(step.name, || {
                format!(
                    "sum of {} batch calls; passes interleave per batch; no per-star timers",
                    record.frame_calls
                )
            });
        }
    }
}
