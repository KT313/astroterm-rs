//! Opt-in, hierarchically grouped diagnostics. Ordinary frame timing never evaluates diagnostic closures.
use super::StepTimes;
use std::io::{self, Write};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineTrace {
    pub presented_frames: u64,
    pub reused_display_frames: u64,
    pub steps: Vec<TraceStep>,
    pub unscoped_diagnostic_seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceStep {
    pub name: &'static str,
    pub depth: usize,
    pub seconds: f64,
    pub details: Vec<String>,
    pub direct_diagnostic_seconds: f64,
    pub(super) parent: Option<usize>,
}

impl StepTimes {
    /// Record only completed rendering; failed writes never call this hook.
    pub fn record_frame_presentation(&mut self, presented: bool) {
        let Some(trace) = &mut self.trace else { return; };
        if presented { trace.presented_frames = trace.presented_frames.saturating_add(1); }
        else { trace.reused_display_frames = trace.reused_display_frames.saturating_add(1); }
    }

    pub fn with_trace(enabled: bool) -> Self {
        Self {
            trace: enabled.then(PipelineTrace::default),
            ..Self::default()
        }
    }

    pub fn trace(&self) -> Option<&PipelineTrace> {
        self.trace.as_ref()
    }

    /// Attach data to the exact checkpoint path in this segment. No formatting or counting when disabled.
    pub fn describe(&mut self, name: &'static str, describe: impl FnOnce() -> String) {
        let checkpoint = self.checkpoint_trace_index(name);
        let Some(trace) = &mut self.trace else {
            return;
        };
        let start = std::time::Instant::now();
        if let Some(index) = checkpoint {
            trace.steps[index].details.push(describe());
        }
        self.record_diagnostic_time(start.elapsed().as_secs_f64());
    }

    pub(super) fn record_diagnostic_time(&mut self, seconds: f64) {
        let checkpoint = self.active_checkpoint_trace_index();
        let trace = self.trace.as_mut().unwrap();
        if let Some(index) = checkpoint {
            trace.steps[index].direct_diagnostic_seconds += seconds;
        } else {
            trace.unscoped_diagnostic_seconds += seconds;
        }
    }

    /// Keep startup diagnostics, but exclude startup costs from the frame-time panel.
    pub fn reset_frame_timings(&mut self) {
        *self.checkpoints = Default::default();
        self.steps.clear();
        self.records.clear();
        self.per_frame = false;
    }

}

impl PipelineTrace {
    pub fn write_report(&self, output: &mut impl Write) -> io::Result<()> {
        self.write_execution_report(output, true)
    }

    fn write_execution_report(&self, output: &mut impl Write, single_frame_header: bool) -> io::Result<()> {
        if single_frame_header {
            writeln!(output, "astroterm --debug-singleframe: execution trace")?;
            writeln!(
                output,
                "Presented frames: {}. Paths are grouped hierarchically; repeated checkpoints are aggregated.",
                self.presented_frames
            )?;
        } else {
            writeln!(output, "Paths are grouped hierarchically; repeated checkpoints are aggregated.")?;
        }
        writeln!(
            output,
            "Times are unsmoothed wall times. Parent times include children and diagnostic overhead; do not sum them."
        )?;
        writeln!(
            output,
            "Counts describe stage inputs/results, not unique visible pixels. Present measures writes/flush, not display completion."
        )?;
        writeln!(
            output,
            "Self/unattributed = parent minus direct child timers and direct diagnostics; includes timer overhead and uncovered work."
        )?;
        writeln!(
            output,
            "Diagnostic work outside stage timers: {:.3} ms",
            self.unscoped_diagnostic_seconds * 1000.0
        )?;
        let mut children = vec![0.0; self.steps.len()];
        for step in &self.steps {
            if let Some(parent) = step.parent {
                children[parent] += step.seconds;
            }
        }
        for (index, step) in self.steps.iter().enumerate() {
            let indent = "  ".repeat(step.depth);
            writeln!(
                output,
                "{:03} {indent}{}: {:.3} ms",
                index + 1,
                step.name,
                step.seconds * 1000.0
            )?;
            if children[index] > 0.0 || step.direct_diagnostic_seconds > 0.0 {
                writeln!(
                    output,
                    "    {indent}direct children={:.3} ms; direct diagnostics={:.3} ms; self/unattributed={:.3} ms",
                    children[index] * 1000.0,
                    step.direct_diagnostic_seconds * 1000.0,
                    (step.seconds - children[index] - step.direct_diagnostic_seconds).max(0.0) * 1000.0
                )?;
            }
            for detail in &step.details {
                writeln!(output, "    {indent}{detail}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_follow_exact_paths_and_remain_lazy_when_disabled_or_absent() {
        let mut disabled = StepTimes::default();
        disabled.describe("Missing", || panic!("disabled diagnostics must stay lazy"));
        let mut times = StepTimes::with_trace(true);
        times.measure("First.Child");
        times.describe("First.Child", || "first child".into());
        times.measure("Second.Child");
        times.describe("Second.Missing", || panic!("absent paths must stay lazy"));
        times.describe("Second.Child", || "second child".into());
        times.measure("END");
        let trace = times.trace().unwrap();
        assert_eq!(trace.steps[1].details, ["first child"]);
        assert_eq!(trace.steps[3].details, ["second child"]);
        assert!(trace.steps[1].direct_diagnostic_seconds > 0.0);
        assert!(trace.steps[1].seconds >= trace.steps[1].direct_diagnostic_seconds);
        let mut report = Vec::new();
        trace.write_report(&mut report).unwrap();
        assert!(String::from_utf8(report).unwrap().contains("Child:"));
    }
}
