//! Borrow raw timing records without copying their payloads or changing the display's smoothed averages.
use super::{StepTimes, TraceStep};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::io;

pub struct CaptureSteps<'a>(&'a [TraceStep]);
impl Serialize for CaptureSteps<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Step<'a> { name: &'a str, parent: Option<usize>, seconds: f64, diagnostic_seconds: f64 }
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for step in self.0 {
            sequence.serialize_element(&Step { name: step.name, parent: step.parent, seconds: step.seconds,
                diagnostic_seconds: step.direct_diagnostic_seconds })?;
        }
        sequence.end()
    }
}

impl StepTimes {
    pub fn capture_steps(&self) -> io::Result<CaptureSteps<'_>> {
        let trace = self.trace.as_ref().ok_or_else(|| io::Error::other("frame capture requires tracing"))?;
        if !trace.active.is_empty() || !self.parents.is_empty() { return Err(io::Error::other("cannot export an active timing scope")); }
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded || self.memory_enabled { return Err(io::Error::other("frame capture cannot share memory-report retention")); }
        for (index, step) in trace.steps.iter().enumerate() {
            if !step.seconds.is_finite() || step.seconds < 0.0 || !step.direct_diagnostic_seconds.is_finite()
                || step.direct_diagnostic_seconds < 0.0 || step.parent.is_some_and(|parent| parent >= index) {
                return Err(io::Error::other("invalid raw timing record"));
            }
        }
        Ok(CaptureSteps(&trace.steps))
    }

    /// Reuse one completed trace's allocation. Runtime caches and running timing averages remain untouched.
    pub fn clear_capture_segment(&mut self) -> io::Result<()> {
        self.capture_steps()?;
        let trace = self.trace.as_mut().unwrap();
        let mut steps = std::mem::take(&mut trace.steps);
        steps.clear();
        *trace = super::PipelineTrace { steps, ..Default::default() };
        #[cfg(feature = "memory-diagnostics")]
        {
            self.memory_epoch = self.memory_epoch.checked_add(1).expect("diagnostic frame handle epoch exhausted");
            self.memory_completed = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "memory-diagnostics")]
    #[test]
    fn bounded_memory_history_cannot_be_discarded_by_frame_capture() {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_run(true);
        times.measure("Startup", || {});
        assert!(times.capture_steps().is_err());
        assert!(times.clear_capture_segment().is_err());
        assert_eq!(times.trace().unwrap().steps.len(), 1);
    }

    #[test]
    fn thousands_of_frames_reuse_one_trace_without_losing_parent_indices() {
        let mut times = StepTimes::with_trace(true);
        let mut capacity = 0;
        for index in 0..3000 {
            times.begin_frame();
            times.measure_steps("Parent", |times| { times.measure("Child", || {}); });
            let value = serde_json::to_value(times.capture_steps().unwrap()).unwrap();
            assert_eq!(value[1]["parent"], 0);
            let current = times.trace().unwrap().steps.capacity();
            if index == 0 { capacity = current; } else { assert_eq!(current, capacity); }
            times.clear_capture_segment().unwrap();
            assert_eq!(times.trace().unwrap().steps.len(), 0);
        }
    }
}
