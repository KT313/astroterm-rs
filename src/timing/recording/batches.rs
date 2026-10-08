//! Aggregate flat batch passes while preserving their existing event and timing boundaries.
use super::StepTimes;
#[cfg(feature = "memory-diagnostics")]
use std::time::Instant;

impl StepTimes {
    pub(super) fn prepare_batch_times(&self) -> Self {
        let mut batches = Self::default();
        batches.begin_frame();
        #[cfg(feature = "memory-diagnostics")]
        { batches.memory_enabled = self.memory_enabled; batches.memory_batch = self.memory_enabled; batches.memory_bounded = self.memory_bounded; batches.memory_epoch = self.memory_epoch; }
        batches
    }

    pub(super) fn record_batch_results(&mut self, batches: &Self) {
        for (step, record) in batches.steps.iter().zip(&batches.records) {
            assert_eq!(step.depth, 0, "batch passes must be flat");
            let seconds = record.frame_seconds.unwrap_or(0.0);
            let index = self.start_trace(step.name);
            self.finish_trace(index, seconds);
            self.record(step.name, seconds);
            #[cfg(feature = "memory-diagnostics")]
            if self.memory_enabled && let Some(index) = index {
                let start = Instant::now();
                let available = if self.memory_bounded { crate::constants::MAX_TRACE_EVENTS.saturating_sub(self.trace.as_ref().unwrap().event_count) } else { usize::MAX };
                let destination = &mut self.trace.as_mut().unwrap().steps[index];
                destination.memory_events = record.memory_events.iter().take(available).cloned().collect();
                destination.memory_omitted = record.memory_events.iter().skip(available).fold(record.memory_omitted, |sum, event| sum.saturating_add(usize::try_from(event.calls).unwrap_or(usize::MAX)));
                destination.invocations = record.frame_calls as u64;
                destination.memory_aggregated = true;
                self.trace.as_mut().unwrap().event_count += record.memory_events.len().min(available);
                self.record_diagnostic_time(start.elapsed().as_secs_f64());
            }
            self.describe(step.name, || {
                format!(
                    "sum of {} batch calls; passes interleave per batch; no per-star timers",
                    record.frame_calls
                )
            });
        }
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_enabled {
            self.record_diagnostic_time(batches.memory_batch_seconds);
            self.registry_omitted = self.registry_omitted.saturating_add(batches.registry_omitted);
        }
    }
}
