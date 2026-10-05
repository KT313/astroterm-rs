//! Feature-enabled event storage and diagnostic-time accounting.
use super::{Access, BufferShape, MemoryEvent, MemoryStepId, Operation, StepTimes, Target};
use crate::timing::formatting::{format_bytes as bytes, format_count as count};
use std::{io::{self, Write}, time::Instant};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMemoryEvent {
    pub event: MemoryEvent,
    /// Number of recorded observations; a batch observation may cover many underlying operations.
    pub calls: u64,
    pub total_elements: Option<usize>,
    pub total_logical_bytes: Option<usize>,
}
impl RecordedMemoryEvent {
    fn new(event: MemoryEvent) -> Self {
        let (total_elements, total_logical_bytes) = event.counts();
        Self { event, calls: 1, total_elements, total_logical_bytes }
    }
    fn merge(&mut self, event: MemoryEvent) {
        let (elements, bytes) = event.counts();
        self.calls = self.calls.saturating_add(1);
        self.total_elements = self.total_elements.zip(elements).and_then(|(a,b)| a.checked_add(b));
        self.total_logical_bytes = self.total_logical_bytes.zip(bytes).and_then(|(a,b)| a.checked_add(b));
        if let (MemoryEvent::Operation { after, .. }, MemoryEvent::Operation { after: latest, .. }) = (&mut self.event, event) { *after = latest; }
    }
}

pub const MAX_MEMORY_EVENTS_PER_STEP: usize = 128;

pub(crate) fn append_event(events: &mut Vec<RecordedMemoryEvent>, omitted: &mut usize, event: MemoryEvent, aggregate: bool) {
    if aggregate && let Some(record) = events.iter_mut().find(|record| record.event.same_kind(event)) {
        record.merge(event);
    } else if events.len() < MAX_MEMORY_EVENTS_PER_STEP {
        events.push(RecordedMemoryEvent::new(event));
    } else {
        *omitted = omitted.saturating_add(1);
    }
}

impl StepTimes {
    /// Enable additional events only for an existing trace. Single-frame tracing alone does not enable memory work.
    pub fn enable_memory_events(&mut self, enabled: bool) { self.memory_enabled = enabled && self.trace.is_some(); }
    pub fn memory_events_enabled(&self) -> bool { self.memory_enabled }

    /// Exact completed invocation, including a flat batch pass. Obtain immediately after its measured call.
    pub fn last_memory_step(&self) -> Option<MemoryStepId> {
        self.memory_enabled.then_some(self.memory_completed).flatten()
    }
    /// Exact enclosing measured scope. Use at a narrow-view construction inside measure_steps.
    pub fn active_memory_step(&self) -> Option<MemoryStepId> {
        if !self.memory_enabled || self.memory_suppressed != 0 { return None; }
        self.trace.as_ref()?.active.last().copied().map(|index| MemoryStepId(Target::Trace(index), self.memory_epoch))
    }

    /// Lazily inspect a small boundary descriptor. Never traverse the root or a per-star cache here.
    pub fn inspect_memory<T>(&mut self, inspect: impl FnOnce() -> T) -> Option<T> {
        if !self.memory_enabled { return None; }
        let start = Instant::now();
        let value = inspect();
        self.charge_memory_time(start.elapsed().as_secs_f64());
        Some(value)
    }
    /// Attach a value-only event to an exact invocation. Disabled/invalid targets do not evaluate the closure.
    pub fn record_memory(&mut self, id: Option<MemoryStepId>, event: impl FnOnce() -> MemoryEvent) {
        if !self.memory_enabled { return; }
        let Some(id) = id else { return; };
        if id.1 != self.memory_epoch { return; }
        let valid = match id.0 {
            Target::Trace(i) => self.trace.as_ref().is_some_and(|trace| i < trace.steps.len()),
            Target::Batch(i) => self.memory_batch && i < self.records.len(),
        };
        if !valid { return; }
        if let Target::Trace(i) = id.0 {
            let trace = self.trace.as_mut().unwrap();
            if self.memory_bounded && (trace.event_count >= crate::timing::run::MAX_TRACE_EVENTS || trace.steps[i].memory_events.len() >= MAX_MEMORY_EVENTS_PER_STEP) {
                trace.steps[i].memory_omitted = trace.steps[i].memory_omitted.saturating_add(1);
                return;
            }
        }
        let start = Instant::now();
        let event = event();
        match id.0 {
            Target::Trace(i) => {
                let trace = self.trace.as_mut().unwrap();
                let step = &mut trace.steps[i];
                let before = step.memory_events.len();
                append_event(&mut step.memory_events, &mut step.memory_omitted, event, false);
                trace.event_count += step.memory_events.len() - before;
            }
            Target::Batch(i) => {
                let record = &mut self.records[i];
                let existing = record.memory_events.iter().any(|record| record.event.same_kind(event));
                if self.memory_bounded && self.memory_batch_events >= crate::timing::run::MAX_TRACE_EVENTS && !existing {
                    record.memory_omitted = record.memory_omitted.saturating_add(1);
                } else {
                    let before = record.memory_events.len();
                    append_event(&mut record.memory_events, &mut record.memory_omitted, event, true);
                    self.memory_batch_events += record.memory_events.len() - before;
                }
            }
        }
        self.charge_memory_time(start.elapsed().as_secs_f64());
    }
    pub(crate) fn charge_memory_time(&mut self, seconds: f64) {
        if self.memory_batch { self.memory_batch_seconds += seconds; }
        else { self.record_diagnostic_time(seconds); }
    }
}

fn shape(value: BufferShape) -> String {
    format!("{:?} indices; len={}; capacity={}; direct payload={}; {:?}", value.domain, count(value.len), count(value.capacity), bytes(value.logical_bytes()), value.quality)
}
fn operation_name(operation: Operation) -> String {
    match operation {
        Operation::Store { value_changed } => format!("cache commit (value {})", if value_changed { "changed" } else { "unchanged" }),
        Operation::Refresh(reason) => format!("refresh required ({reason:?})"),
        Operation::RefreshUnknown => "refresh required (reason unknown)".into(),
        Operation::Compare => "comparison invoked (examined elements not inferred)".into(),
        Operation::Move => "ownership transfer (not a payload copy)".into(),
        Operation::Output => "completed writer operation (not display completion)".into(),
        Operation::Write => "buffer write".into(),
        Operation::Clear => "clear/reset contents (backing storage may remain)".into(),
        other => format!("{other:?}"),
    }
}

pub(crate) fn write_events(events: &[RecordedMemoryEvent], omitted: usize, aggregate: bool, output: &mut impl Write, indent: &str) -> io::Result<()> {
    if aggregate && !events.is_empty() {
        writeln!(output, "    {indent}memory: interleaved batch totals; repeated grants/operations are counted repeatedly; shapes show first-before/last-after, not a whole-catalog pass")?;
    }
    for record in events {
        match record.event {
            MemoryEvent::Borrow { buffer, access, shape: value } => {
                let access = match access { Access::ReadOnly => "read-only", Access::Writable => "writable" };
                writeln!(output, "    {indent}memory: {access} grant to {buffer:?}; {}{}", if aggregate { "first grant shape: " } else { "" }, shape(value))?;
            }
            MemoryEvent::Operation { buffer, operation, before, after, quality, .. } => {
                writeln!(output, "    {indent}memory: {} on {buffer:?}; {quality:?}", operation_name(operation))?;
                if let Some(before) = before { writeln!(output, "      {indent}before: {}", shape(before))?; }
                if let Some(after) = after { writeln!(output, "      {indent}after:  {}", shape(after))?; }
            }
        }
        let payload_label = match record.event {
            MemoryEvent::Borrow { .. } => "logical granted payload",
            MemoryEvent::Operation { operation: Operation::Copy, .. } => "logical copied payload",
            MemoryEvent::Operation { operation: Operation::Move, .. } => "logical transferred payload",
            MemoryEvent::Operation { operation: Operation::Output, .. } => "submitted payload",
            _ => "logical payload",
        };
        writeln!(output, "      {indent}event observations={}; logical elements={}; {payload_label}={}", record.calls, count(record.total_elements), bytes(record.total_logical_bytes))?;
    }
    if omitted != 0 { writeln!(output, "    {indent}memory events omitted={omitted} (per-step/segment bound)")?; }
    Ok(())
}

crate::cache::buffers::report_flat!(RecordedMemoryEvent);
