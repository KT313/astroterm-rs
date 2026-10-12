//! Opt-in, execution-ordered diagnostics. Ordinary frame timing never evaluates diagnostic closures.
use crate::rows::row_columns;
use super::StepTimes;
use std::io::{self, Write};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineTrace {
    pub presented_frames: u64,
    pub reused_display_frames: u64,
    pub steps: Vec<TraceStep>,
    pub(super) active: Vec<usize>,
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
row_columns!(TraceStep { name, depth, seconds, details, direct_diagnostic_seconds, parent, .. });

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

    /// Attach data to the latest invocation in this scope. No formatting or counting when disabled.
    pub fn describe(&mut self, name: &'static str, describe: impl FnOnce() -> String) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        let start = std::time::Instant::now();
        let parent = trace.active.last().copied();
        if let Some(step) = trace
            .steps
            .iter_mut()
            .rev()
            .find(|s| s.name == name && s.parent == parent)
        {
            step.details.push(describe());
        }
        self.record_diagnostic_time(start.elapsed().as_secs_f64());
    }

    pub(super) fn record_diagnostic_time(&mut self, seconds: f64) {
        let trace = self.trace.as_mut().unwrap();
        if let Some(&index) = trace.active.last() {
            trace.steps[index].direct_diagnostic_seconds += seconds;
        } else {
            trace.unscoped_diagnostic_seconds += seconds;
        }
    }

    pub(super) fn push_trace_scope(&mut self, index: Option<usize>) {
        if let Some(index) = index {
            self.trace.as_mut().unwrap().active.push(index);
        }
    }

    pub(super) fn pop_trace_scope(&mut self, index: Option<usize>) {
        if index.is_some() {
            self.trace.as_mut().unwrap().active.pop();
        }
    }

    /// Keep startup diagnostics, but exclude startup costs from the frame-time panel.
    pub fn reset_frame_timings(&mut self) {
        self.steps.clear();
        self.records.clear();
        self.per_frame = false;
    }

    pub(super) fn start_trace(&mut self, name: &'static str) -> Option<usize> {
        let trace = self.trace.as_mut()?;
        let index = trace.steps.len();
        trace.steps.push(TraceStep {
            name,
            depth: trace.active.last().map_or(0, |&parent| trace.steps[parent].depth + 1),
            seconds: 0.0,
            details: Vec::new(),
            direct_diagnostic_seconds: 0.0,
            parent: trace.active.last().copied(),
        });
        Some(index)
    }

    pub(super) fn finish_trace(&mut self, index: Option<usize>, seconds: f64) {
        if let Some(index) = index {
            self.trace.as_mut().unwrap().steps[index].seconds = seconds;
        }
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
                "Presented frames: {}. Invocation/start order; repeated calls are separate except explicitly aggregated batch passes.",
                self.presented_frames
            )?;
        } else {
            writeln!(output, "Execution trace in invocation/start order; repeated calls are separate except explicitly aggregated batch passes.")?;
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
    fn diagnostic_cost_is_charged_to_enclosing_scope_and_scopes_do_not_leak() {
        let mut times = StepTimes::with_trace(true);
        times.measure_steps("First", |times| {
            times.measure("Child", || ());
            times.describe("Child", || "first only".into());
        });
        times.measure_steps("Second", |times| {
            times.describe("Child", || panic!("no Child in this invocation"));
        });
        let trace = times.trace().unwrap();
        assert_eq!(trace.steps[1].details, ["first only"]);
        assert!(trace.steps[0].direct_diagnostic_seconds > 0.0);
        assert_eq!(trace.steps[1].direct_diagnostic_seconds, 0.0);
        assert!(trace.steps[0].seconds >= trace.steps[1].seconds + trace.steps[0].direct_diagnostic_seconds);
    }

    #[test]
    fn records_each_call_in_start_order_with_lazy_scoped_details() {
        let mut disabled = StepTimes::default();
        disabled.describe("unused", || panic!("must stay lazy"));
        let mut times = StepTimes::with_trace(true);
        times.measure_steps("Parent", |times| {
            times.measure("Child", || 1);
            times.describe("Child", || "first input=2 output=1".into());
            times.measure("Child", || 2);
            times.describe("Child", || "second input=3 output=2".into());
        });
        times.describe("Parent", || "done".into());
        let trace = times.trace().unwrap();
        assert_eq!(
            trace.steps.iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(),
            [("Parent", 0), ("Child", 1), ("Child", 1)]
        );
        assert_eq!(trace.steps[1].details, ["first input=2 output=1"]);
        assert_eq!(trace.steps[2].details, ["second input=3 output=2"]);
        let mut report = Vec::new();
        trace.write_report(&mut report).unwrap();
        assert!(String::from_utf8(report).unwrap().contains("003   Child:"));
        times.reset_frame_timings();
        times.begin_frame();
        assert!(times.steps().is_empty());
        assert_eq!(times.trace().unwrap().steps.len(), 3);
    }
}
