//! Opt-in, execution-ordered diagnostics. Ordinary frame timing never evaluates diagnostic closures.
use super::StepTimes;
use std::io::{self, Write};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineTrace {
    pub steps: Vec<TraceStep>,
    active: Vec<usize>,
    pub unscoped_diagnostic_seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceStep {
    pub name: &'static str,
    pub depth: usize,
    pub seconds: f64,
    pub details: Vec<String>,
    pub direct_diagnostic_seconds: f64,
    parent: Option<usize>,
}

impl StepTimes {
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

    /// Account a diagnostic scan in its actual enclosing timer, while descriptions still attach to their targets.
    pub(crate) fn measure_diagnostics(&mut self, run: impl FnOnce(&mut Self)) {
        if self.trace.is_none() {
            return;
        }
        let start = std::time::Instant::now();
        let before = self.current_diagnostic_seconds();
        run(self);
        let nested = self.current_diagnostic_seconds() - before;
        self.record_diagnostic_time((start.elapsed().as_secs_f64() - nested).max(0.0));
    }

    fn current_diagnostic_seconds(&self) -> f64 {
        let trace = self.trace.as_ref().unwrap();
        trace.active.last().map_or(trace.unscoped_diagnostic_seconds, |&i| {
            trace.steps[i].direct_diagnostic_seconds
        })
    }

    fn record_diagnostic_time(&mut self, seconds: f64) {
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
            depth: self.parents.len(),
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
        writeln!(output, "astroterm --debug-singleframe: execution trace")?;
        writeln!(
            output,
            "Presented frames: {}. Invocation/start order; repeated calls are separate except explicitly aggregated batch passes.",
            self.steps.iter().filter(|s| s.name == "Present").count()
        )?;
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
            times.measure_diagnostics(|times| times.describe("Child", || "first only".into()));
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
    fn batch_totals_are_summed_with_bounded_trace_size() {
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        times.measure_batches("Batches", |batch| {
            for _ in 0..100 {
                batch.measure("Read", || 1);
                batch.measure("Compute", || 2);
            }
        });
        let trace = times.trace().unwrap();
        assert_eq!(trace.steps.len(), 3);
        assert_eq!(trace.steps[1].parent, Some(0));
        assert_eq!(trace.steps[2].parent, Some(0));
        assert!(trace.steps[1].details[0].contains("sum of 100 batch calls"));
        assert!(trace.steps[0].seconds >= trace.steps[1].seconds + trace.steps[2].seconds);
        assert_eq!(times.steps()[1].average_seconds, trace.steps[1].seconds);
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
