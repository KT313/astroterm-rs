//! Opt-in, execution-ordered diagnostics. Ordinary frame timing never evaluates diagnostic closures.
use crate::rows::row_columns;
use super::StepTimes;
use std::io::{self, Write};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineTrace {
    pub steps: Vec<TraceStep>,
    #[cfg(feature = "memory-diagnostics")]
    pub memory_snapshots: Vec<crate::cache::InventorySnapshot>,
    pub(super) active: Vec<usize>,
    pub unscoped_diagnostic_seconds: f64,
    #[cfg(feature = "memory-diagnostics")]
    pub bounds: super::run::TraceBounds,
    #[cfg(feature = "memory-diagnostics")]
    pub(super) event_count: usize,
    #[cfg(feature = "memory-diagnostics")]
    pub(super) detail_count: usize,
    #[cfg(feature = "memory-diagnostics")]
    pub(super) detail_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceStep {
    pub name: &'static str,
    pub depth: usize,
    pub seconds: f64,
    pub details: Vec<String>,
    pub direct_diagnostic_seconds: f64,
    pub(super) parent: Option<usize>,
    #[cfg(feature = "memory-diagnostics")]
    pub invocations: u64,
    #[cfg(feature = "memory-diagnostics")]
    pub memory_events: Vec<super::memory::RecordedMemoryEvent>,
    #[cfg(feature = "memory-diagnostics")]
    pub memory_omitted: usize,
    #[cfg(feature = "memory-diagnostics")]
    pub memory_aggregated: bool,
    #[cfg(feature = "memory-diagnostics")]
    pub(super) details_superseded: bool, // a later invocation with this name and parent was omitted
}
row_columns!(TraceStep { name, depth, seconds, details, direct_diagnostic_seconds, parent, .. });

impl StepTimes {
    /// Keep one inventory per bounded segment (two for legacy single-frame traces); omitted callbacks stay lazy.
    #[cfg(feature = "memory-diagnostics")]
    pub fn capture_memory(&mut self, capture: impl FnOnce(&Self) -> crate::cache::InventorySnapshot) {
        let Some(trace) = &mut self.trace else { return; };
        let limit = if self.memory_bounded { super::run::MAX_TRACE_INVENTORIES } else { 2 };
        if trace.memory_snapshots.len() >= limit {
            trace.bounds.omitted_inventories = trace.bounds.omitted_inventories.saturating_add(1);
            return;
        }
        let start = std::time::Instant::now();
        let mut snapshot = capture(self);
        snapshot.capture_seconds = start.elapsed().as_secs_f64();
        self.trace.as_mut().unwrap().memory_snapshots.push(snapshot);
        self.record_diagnostic_time(start.elapsed().as_secs_f64());
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
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && (self.memory_suppressed != 0 || trace.detail_count >= super::run::MAX_TRACE_DETAILS || trace.detail_bytes >= super::run::MAX_TRACE_TEXT_BYTES) {
            trace.bounds.omitted_details = trace.bounds.omitted_details.saturating_add(1);
            return;
        }
        let parent = trace.active.last().copied();
        if let Some(step) = trace
            .steps
            .iter_mut()
            .rev()
            .find(|s| s.name == name && s.parent == parent)
        {
            #[cfg(feature = "memory-diagnostics")]
            if step.details_superseded {
                trace.bounds.omitted_details = trace.bounds.omitted_details.saturating_add(1);
                return; // an omitted invocation must not attach its details to an older retained call
            }
            let detail = describe();
            #[cfg(feature = "memory-diagnostics")]
            let detail = if self.memory_bounded {
                let limit = super::run::MAX_DETAIL_BYTES.min(super::run::MAX_TRACE_TEXT_BYTES - trace.detail_bytes);
                let mut end = detail.len().min(limit);
                while !detail.is_char_boundary(end) { end -= 1; }
                trace.bounds.truncated_text_bytes = trace.bounds.truncated_text_bytes.saturating_add(detail.len() - end);
                trace.detail_count += 1;
                trace.detail_bytes += end;
                detail[..end].to_owned() // discard an oversized temporary allocation instead of retaining its capacity
            } else { detail };
            step.details.push(detail);
        } else {
            #[cfg(feature = "memory-diagnostics")]
            if self.memory_bounded { trace.bounds.omitted_details = trace.bounds.omitted_details.saturating_add(1); }
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

    pub(super) fn record_diagnostic_time(&mut self, seconds: f64) {
        let trace = self.trace.as_mut().unwrap();
        if let Some(&index) = trace.active.last() {
            trace.steps[index].direct_diagnostic_seconds += seconds;
        } else {
            trace.unscoped_diagnostic_seconds += seconds;
        }
    }

    pub(super) fn push_trace_scope(&mut self, index: Option<usize>) {
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && index.is_none() { self.memory_suppressed += 1; }
        if let Some(index) = index {
            self.trace.as_mut().unwrap().active.push(index);
        }
    }

    pub(super) fn pop_trace_scope(&mut self, index: Option<usize>) {
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && index.is_none() { self.memory_suppressed -= 1; }
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
        #[cfg(feature = "memory-diagnostics")]
        if self.memory_bounded && (trace.steps.len() >= super::run::MAX_TRACE_STEPS || trace.active.len() >= super::run::MAX_TRACE_DEPTH || self.memory_suppressed != 0) {
            if self.memory_suppressed == 0 {
                let parent = trace.active.last().copied();
                if let Some(step) = trace.steps.iter_mut().rev().find(|step| step.name == name && step.parent == parent) {
                    step.details_superseded = true; // this retained row is no longer the latest invocation for describe(name)
                }
            } // suppressed descendants have an omitted parent, not the last retained active scope
            trace.bounds.omitted_steps = trace.bounds.omitted_steps.saturating_add(1);
            self.memory_completed = None;
            return None;
        }
        let index = trace.steps.len();
        trace.steps.push(TraceStep {
            name,
            depth: trace.active.last().map_or(0, |&parent| trace.steps[parent].depth + 1),
            seconds: 0.0,
            #[cfg(feature = "memory-diagnostics")]
            invocations: 1,
            details: Vec::new(),
            direct_diagnostic_seconds: 0.0,
            parent: trace.active.last().copied(),
            #[cfg(feature = "memory-diagnostics")]
            memory_events: Vec::new(),
            #[cfg(feature = "memory-diagnostics")]
            memory_omitted: 0,
            #[cfg(feature = "memory-diagnostics")]
            memory_aggregated: false,
            #[cfg(feature = "memory-diagnostics")]
            details_superseded: false,
        });
        Some(index)
    }

    pub(super) fn finish_trace(&mut self, index: Option<usize>, seconds: f64) {
        if let Some(index) = index {
            self.trace.as_mut().unwrap().steps[index].seconds = seconds;
            #[cfg(feature = "memory-diagnostics")]
            if self.memory_enabled { self.memory_completed = Some(super::memory::MemoryStepId(super::memory::Target::Trace(index), self.memory_epoch)); }
        }
    }
}

impl PipelineTrace {
    pub fn write_report(&self, output: &mut impl Write) -> io::Result<()> {
        self.write_execution_report(output, true)
    }

    #[cfg(feature = "memory-diagnostics")]
    pub(super) fn write_segment(&self, output: &mut impl Write) -> io::Result<()> { self.write_execution_report(output, false) }

    fn write_execution_report(&self, output: &mut impl Write, single_frame_header: bool) -> io::Result<()> {
        #[cfg(feature = "memory-diagnostics")]
        self.bounds.write_report(output)?;
        if single_frame_header {
            writeln!(output, "astroterm --debug-singleframe: execution trace")?;
            writeln!(
                output,
                "Presented frames: {}. Invocation/start order; repeated calls are separate except explicitly aggregated batch passes.",
                self.steps.iter().filter(|s| s.name == "Present").count()
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
        #[cfg(feature = "memory-diagnostics")]
        let mut memory_report_seconds = 0.0;
        #[cfg(feature = "memory-diagnostics")]
        let has_memory_events = self.steps.iter().any(|step| !step.memory_events.is_empty() || step.memory_omitted > 0);
        #[cfg(feature = "memory-diagnostics")]
        if has_memory_events {
            writeln!(output, "Memory events cover instrumented operations only. Grants describe permission, not actual reads/writes. Bytes are logical direct payload, not RAM traffic; nested allocations are excluded unless stated. Capacity changes do not prove relocation. Clear is not free. Unknown remains unknown. Small inline diagnostic counters stay in pass timings; descriptor work is charged separately.")?;
        }
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
            #[cfg(feature = "memory-diagnostics")]
            if !step.memory_events.is_empty() || step.memory_omitted > 0 {
                let start = std::time::Instant::now();
                super::memory::write_events(&step.memory_events, step.memory_omitted, step.memory_aggregated, output, &indent)?;
                memory_report_seconds += start.elapsed().as_secs_f64();
            }
            for detail in &step.details {
                writeln!(output, "    {indent}{detail}")?;
            }
        }
        #[cfg(feature = "memory-diagnostics")]
        if has_memory_events { writeln!(output, "Memory event report formatting/output: {:.3} ms (after frame; outside pipeline timings)", memory_report_seconds * 1000.0)?; }
        Ok(())
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(TraceStep { details, memory_events });
#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for PipelineTrace {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use crate::cache::{report_field, Quality};
        report_field(sink, "steps", &self.steps);
        report_field(sink, "active", &self.active);
        if sink.enter("stored_memory_reports", std::mem::size_of_val(&self.memory_snapshots)) {
            sink.payload(self.memory_snapshots.len(), self.memory_snapshots.capacity(), std::mem::size_of::<crate::cache::InventorySnapshot>(), Quality::ExactPayload, "snapshot headers");
            for report in &self.memory_snapshots {
                if let Some((used, reserved)) = report.used_bytes().zip(report.retained_bytes()) { sink.payload(used, reserved, 1, Quality::ExactPayload, "captured descriptors and path bytes/capacities"); }
                else { sink.unknown("report byte count overflow"); }
            }
            sink.leave();
        }
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
