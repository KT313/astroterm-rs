//! Bounded startup, latest-frame and partial-frame evidence for opt-in continuous diagnostics.
use crate::constants::{MAX_AGGREGATE_PATHS, MAX_DETAIL_BYTES, MAX_TIMING_PATHS, MAX_TRACE_DEPTH, MAX_TRACE_DETAILS, MAX_TRACE_EVENTS, MAX_TRACE_INVENTORIES, MAX_TRACE_STEPS, MAX_TRACE_TEXT_BYTES};
use super::{PipelineTrace, StepTimes};
use std::io::{self, Write};


#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceBounds {
    pub omitted_steps: usize,
    pub omitted_details: usize,
    pub truncated_text_bytes: usize,
    pub omitted_inventories: usize,
}
impl TraceBounds {
    pub(super) fn write_report(&self, output: &mut impl Write) -> io::Result<()> {
        if self.omitted_steps != 0 || self.omitted_details != 0 || self.truncated_text_bytes != 0 || self.omitted_inventories != 0 {
            writeln!(output, "Trace truncation: omitted steps={}; omitted details={}; discarded UTF-8 detail bytes={}; omitted inventories={}", self.omitted_steps, self.omitted_details, self.truncated_text_bytes, self.omitted_inventories)?;
        }
        Ok(())
    }
}

/// Sums and actual invocation counts over completed frames only; no second moving-average registry.
#[derive(Clone, Debug, PartialEq)]
pub struct StepAggregate {
    pub path: Vec<&'static str>,
    pub invocations: u64,
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryFrame {
    pub utc: Option<f64>,
    pub tt: Option<f64>,
    pub elapsed_seconds: f64,
    pub trace: PipelineTrace,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryRun {
    pub cache_enabled: bool,
    pub startup: Option<PipelineTrace>,
    pub latest: Option<MemoryFrame>,
    pub completed_frames: u64,
    pub current_time: Option<(f64, f64)>,
    pub frame_active: bool,
    pub aggregates: Vec<StepAggregate>,
    pub omitted_aggregate_steps: u64,
    pub completed_omitted_steps: u64,
    pub completed_omitted_events: u64,
    pub completed_omitted_details: u64,
    pub completed_omitted_inventories: u64,
    pub completed_truncated_text_bytes: u64,
}

impl StepTimes {
    pub(super) fn prepare_bounded_history(&mut self) {
        assert!(self.parents.is_empty(), "enable run diagnostics outside a measured scope");
        self.trace.get_or_insert_with(PipelineTrace::default).bound_existing();
        self.registry_omitted = self.registry_omitted.saturating_add(self.steps.len().saturating_sub(MAX_TIMING_PATHS) as u64);
        self.steps.truncate(MAX_TIMING_PATHS);
        self.records.truncate(MAX_TIMING_PATHS);
        self.steps.shrink_to_fit();
        self.records.shrink_to_fit();
    }
}

impl MemoryRun {
    pub(super) fn new(cache_enabled: bool) -> Self {
        Self { cache_enabled, startup: None, latest: None, completed_frames: 0, current_time: None, frame_active: false,
            aggregates: Vec::new(), omitted_aggregate_steps: 0, completed_omitted_steps: 0, completed_omitted_events: 0,
            completed_omitted_details: 0, completed_omitted_inventories: 0, completed_truncated_text_bytes: 0 }
    }
}


impl PipelineTrace {
    /// Activation can follow a few startup records. Bound those too instead of keeping a hidden unbounded prefix.
    fn bound_existing(&mut self) {
        assert!(self.active.is_empty(), "enable run diagnostics outside a measured scope");
        let step_limit = self.steps.len().min(MAX_TRACE_STEPS);
        let keep = self.steps[..step_limit].iter().position(|step| step.depth >= MAX_TRACE_DEPTH).unwrap_or(step_limit); // omit the over-depth subtree suffix so retained parent indices stay valid
        let (retained, omitted) = self.steps.split_at_mut(keep);
        for dropped in omitted {
            if dropped.parent.is_some_and(|parent| parent >= keep) { continue; } // children of omitted parents cannot supersede retained siblings
            if let Some(step) = retained.iter_mut().rev().find(|step| step.name == dropped.name && step.parent == dropped.parent) {
                step.details_superseded = true; // preserve old details, but do not attach new details for the truncated invocation
            }
        }
        self.bounds.omitted_steps = self.bounds.omitted_steps.saturating_add(self.steps.len() - keep);
        self.steps.truncate(keep);
        self.steps.shrink_to_fit();
        self.bounds.omitted_inventories = self.bounds.omitted_inventories.saturating_add(self.memory_snapshots.len().saturating_sub(MAX_TRACE_INVENTORIES));
        self.memory_snapshots.truncate(MAX_TRACE_INVENTORIES);
        self.memory_snapshots.shrink_to_fit();
        self.event_count = 0;
        self.detail_count = 0;
        self.detail_bytes = 0;
        for step in &mut self.steps {
            let keep = step.memory_events.len().min(crate::constants::MAX_MEMORY_EVENTS_PER_STEP).min(MAX_TRACE_EVENTS - self.event_count);
            step.memory_omitted = step.memory_omitted.saturating_add(step.memory_events.len() - keep);
            step.memory_events.truncate(keep);
            step.memory_events.shrink_to_fit();
            self.event_count += keep;
            let mut details = Vec::new();
            for detail in step.details.drain(..) {
                if self.detail_count == MAX_TRACE_DETAILS || self.detail_bytes == MAX_TRACE_TEXT_BYTES {
                    self.bounds.omitted_details = self.bounds.omitted_details.saturating_add(1);
                    continue;
                }
                let mut end = detail.len().min(MAX_DETAIL_BYTES).min(MAX_TRACE_TEXT_BYTES - self.detail_bytes);
                while !detail.is_char_boundary(end) { end -= 1; }
                self.bounds.truncated_text_bytes = self.bounds.truncated_text_bytes.saturating_add(detail.len() - end);
                details.push(detail[..end].to_owned());
                self.detail_count += 1;
                self.detail_bytes += end;
            }
            step.details = details;
        }
    }
}

pub(super) fn accumulate_completed(run: &mut MemoryRun, trace: &PipelineTrace) {
    run.completed_omitted_inventories = run.completed_omitted_inventories.saturating_add(trace.bounds.omitted_inventories as u64);
    run.completed_omitted_steps = run.completed_omitted_steps.saturating_add(trace.bounds.omitted_steps as u64);
    run.completed_omitted_details = run.completed_omitted_details.saturating_add(trace.bounds.omitted_details as u64);
    run.completed_truncated_text_bytes = run.completed_truncated_text_bytes.saturating_add(trace.bounds.truncated_text_bytes as u64);
    for (index, step) in trace.steps.iter().enumerate() {
        run.completed_omitted_events = run.completed_omitted_events.saturating_add(step.memory_omitted as u64);
        let mut path = Vec::new();
        let mut current = Some(index);
        while let Some(index) = current { path.push(trace.steps[index].name); current = trace.steps[index].parent; }
        path.reverse();
        if let Some(aggregate) = run.aggregates.iter_mut().find(|aggregate| aggregate.path == path) {
            aggregate.invocations = aggregate.invocations.saturating_add(step.invocations);
            aggregate.seconds += step.seconds;
        } else if run.aggregates.len() < MAX_AGGREGATE_PATHS {
            run.aggregates.push(StepAggregate { path, invocations: step.invocations, seconds: step.seconds });
        } else {
            run.omitted_aggregate_steps = run.omitted_aggregate_steps.saturating_add(1);
        }
    }
}

crate::cache::report_fields!(MemoryFrame { trace });
crate::cache::report_fields!(MemoryRun { startup, latest, aggregates });
crate::cache::report_fields!(StepAggregate { path });

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_boundaries_keep_startup_latest_and_partial_separate() {
        let mut times = StepTimes::with_trace(true);
        times.measure("Startup", || ());
        times.enable_memory_run(true);
        for frame in 0..3 {
            times.begin_memory_frame();
            times.set_memory_frame_time(frame as f64, frame as f64);
            times.measure("Present", || ());
            times.complete_memory_frame(0.01);
        }
        times.begin_memory_frame();
        times.measure("Failed work", || ());
        let run = times.memory_run().unwrap();
        assert_eq!(run.completed_frames, 3);
        assert_eq!(run.startup.as_ref().unwrap().steps[0].name, "Startup");
        assert_eq!(run.latest.as_ref().unwrap().utc, Some(2.0));
        assert_eq!(times.trace().unwrap().steps[0].name, "Failed work");
        times.cancel_memory_frame();
        assert_eq!(times.memory_run().unwrap().completed_frames, 3);
    }

    fn event() -> super::super::memory::MemoryEvent {
        use super::super::memory::{BufferId, MemoryEvent, Operation};
        MemoryEvent::unknown_operation(BufferId::StellarScratch, Operation::Clear)
    }

    #[test]
    fn thousands_of_frames_retain_only_startup_latest_and_partial() {
        let mut times = StepTimes::default();
        times.enable_memory_run(false);
        times.measure("Startup", || ());
        times.describe("Startup", || "catalog loaded".into());
        for frame in 0..3000 {
            times.begin_frame();
            times.begin_memory_frame();
            times.set_memory_frame_time(frame as f64, frame as f64 + 0.1);
            times.measure_steps("Observation", |times| {
                times.measure("Motion", || ());
                times.record_memory(times.last_memory_step(), event);
            });
            times.complete_memory_frame(0.01);
            let run = times.memory_run().unwrap();
            assert_eq!(run.startup.as_ref().unwrap().steps.len(), 1);
            assert_eq!(run.latest.as_ref().unwrap().trace.steps.len(), 2);
            assert_eq!(run.aggregates.len(), 2);
            assert!(times.trace().unwrap().steps.is_empty());
            assert_eq!(times.steps.len(), 3);
        }
        let run = times.memory_run().unwrap();
        assert_eq!(run.completed_frames, 3000);
        assert_eq!(run.latest.as_ref().unwrap().utc, Some(2999.0));
        assert!(run.aggregates.iter().all(|aggregate| aggregate.invocations == 3000));
    }

    #[test]
    fn stale_frame_ids_are_lazy_and_cannot_attach_to_reused_indices() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.measure("Startup", || ());
        let startup = times.last_memory_step();
        times.begin_memory_frame();
        times.measure("First", || ());
        times.record_memory(startup, || panic!("stale startup id"));
        let first = times.last_memory_step();
        times.complete_memory_frame(0.0);
        times.begin_memory_frame();
        times.measure("Second", || ());
        times.record_memory(first, || panic!("stale frame id"));
        times.record_memory(times.last_memory_step(), event);
        assert_eq!(times.trace().unwrap().steps[0].memory_events.len(), 1);
    }

    fn nested(times: &mut StepTimes, depth: usize) {
        if depth == 0 { return; }
        times.measure_steps("Nested", |times| nested(times, depth - 1));
    }

    #[test]
    fn steps_depth_and_registry_have_independent_explicit_caps() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        nested(&mut times, MAX_TRACE_DEPTH + 10);
        assert_eq!(times.trace().unwrap().steps.len(), MAX_TRACE_DEPTH);
        assert_eq!(times.trace().unwrap().bounds.omitted_steps, 10);
        assert!(times.parents.is_empty());
        assert!(times.trace().unwrap().active.is_empty());
        for _ in 0..MAX_TRACE_STEPS + 10 { times.measure("Repeated", || ()); }
        let trace = times.trace().unwrap();
        assert_eq!(trace.steps.len(), MAX_TRACE_STEPS);
        assert_eq!(trace.steps.capacity(), MAX_TRACE_STEPS);
        for (index, step) in trace.steps.iter().enumerate() {
            assert!(step.depth < MAX_TRACE_DEPTH);
            assert!(step.parent.is_none_or(|parent| parent < index));
        }
        assert!(times.steps.len() <= MAX_TIMING_PATHS);
    }

    #[test]
    fn detail_strings_bound_bytes_count_and_utf8_without_retaining_large_capacity() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.measure("Data", || ());
        times.describe("Data", || "天".repeat(MAX_DETAIL_BYTES));
        let first = &times.trace().unwrap().steps[0].details[0];
        assert_eq!(first.len(), MAX_DETAIL_BYTES / 3 * 3);
        assert!(first.capacity() <= MAX_DETAIL_BYTES);
        for _ in 0..MAX_TRACE_DETAILS { times.describe("Data", String::new); }
        times.describe("Data", || panic!("count cap must stay lazy"));
        assert_eq!(times.trace().unwrap().detail_count, MAX_TRACE_DETAILS);
        assert!(times.trace().unwrap().bounds.omitted_details > 0);
        times.begin_memory_frame();
        times.measure("Data", || ());
        for _ in 0..MAX_TRACE_TEXT_BYTES / MAX_DETAIL_BYTES {
            times.describe("Data", || "x".repeat(MAX_DETAIL_BYTES));
        }
        times.describe("Data", || panic!("byte cap must stay lazy"));
        assert_eq!(times.trace().unwrap().detail_bytes, MAX_TRACE_TEXT_BYTES);
        assert_eq!(times.trace().unwrap().bounds.omitted_details, 1);
    }

    #[test]
    fn event_caps_apply_per_step_and_segment_without_evaluating_denied_events() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        for _ in 0..MAX_TRACE_EVENTS / crate::constants::MAX_MEMORY_EVENTS_PER_STEP {
            times.measure("Data", || ());
            for _ in 0..crate::constants::MAX_MEMORY_EVENTS_PER_STEP { times.record_memory(times.last_memory_step(), event); }
            times.record_memory(times.last_memory_step(), || panic!("per-step cap"));
        }
        times.measure("No more events", || ());
        times.record_memory(times.last_memory_step(), || panic!("segment cap"));
        let trace = times.trace().unwrap();
        assert_eq!(trace.event_count, MAX_TRACE_EVENTS);
        assert_eq!(trace.steps.last().unwrap().memory_omitted, 1);
        assert_eq!(trace.steps.iter().map(|step| step.memory_events.len()).sum::<usize>(), MAX_TRACE_EVENTS);
    }

    #[test]
    fn aggregate_paths_are_bounded_and_batch_invocations_are_real_calls() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.begin_memory_frame();
        times.measure_batches("Stellar batches", |batch| {
            for _ in 0..100 {
                batch.measure("Read", || ());
                batch.record_memory(batch.last_memory_step(), event);
            }
        });
        times.complete_memory_frame(0.01);
        let run = times.memory_run().unwrap();
        assert_eq!(run.aggregates[0].invocations, 1);
        assert_eq!(run.aggregates[1].invocations, 100);
        assert_eq!(run.latest.as_ref().unwrap().trace.steps[1].memory_events[0].calls, 100);
        // Leaked labels stand in for distinct static instrumentation paths; they are test fixture data, not trace storage.
        for index in 0..MAX_AGGREGATE_PATHS + 10 {
            let label: &'static str = Box::leak(format!("Step {index}").into_boxed_str());
            times.begin_memory_frame();
            times.measure(label, || ());
            times.complete_memory_frame(0.0);
        }
        assert_eq!(times.memory_run().unwrap().aggregates.len(), MAX_AGGREGATE_PATHS);
        assert!(times.memory_run().unwrap().omitted_aggregate_steps > 0);
        assert_eq!(times.steps.len(), MAX_TIMING_PATHS);
        assert!(times.registry_omitted > 0);
    }

    #[test]
    fn report_distinguishes_no_frame_and_failed_frame_without_counting_present_timer() {
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.measure("Startup", || ());
        times.begin_memory_frame();
        times.cancel_memory_frame();
        let mut output = Vec::new();
        times.write_memory_run_report(&mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("completed frames=0"));
        assert!(text.contains("Startup:"));
        assert!(!text.contains("Incomplete frame:"));
        times.begin_memory_frame();
        times.set_memory_frame_time(1.0, 2.0);
        let failed: Result<(), &str> = times.measure("Present", || Err("writer failed"));
        assert!(failed.is_err());
        let mut output = Vec::new();
        times.write_memory_run_report(&mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Incomplete frame:"));
        assert!(text.contains("completed frames=0"));
        assert!(!text.contains("Presented frames: 1"));
        assert!(times.memory_run().unwrap().aggregates.is_empty());
    }

    #[test]
    fn activation_bounds_existing_startup_and_keeps_smoothing_on_frame_boundaries() {
        let mut times = StepTimes::with_trace(true);
        for _ in 0..MAX_TRACE_STEPS + 1 { times.measure("Startup", || ()); }
        times.enable_memory_run(true);
        assert_eq!(times.trace().unwrap().steps.len(), MAX_TRACE_STEPS);
        assert_eq!(times.trace().unwrap().bounds.omitted_steps, 1);
        times.reset_frame_timings();
        times.begin_frame();
        times.record("Work", 1.0);
        times.begin_memory_frame();
        times.complete_memory_frame(0.0);
        assert_eq!(times.steps()[0].average_seconds, 1.0);
        times.begin_frame();
        times.begin_memory_frame();
        times.record("Work", 3.0);
        assert!((times.steps()[0].average_seconds - 1.1).abs() < 1e-12);
    }


    #[test]
    fn batch_transients_and_merged_events_share_segment_bound() {
        use super::super::memory::{BufferId, MemoryEvent, Operation};
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.begin_memory_frame();
        times.measure("Prefix", || ());
        times.record_memory(times.last_memory_step(), event);
        let operations = [Operation::Build, Operation::Clear, Operation::Copy, Operation::Map, Operation::Append,
            Operation::Reserve, Operation::Reuse, Operation::Compare, Operation::Move, Operation::Write,
            Operation::Output, Operation::Release, Operation::Store { value_changed: true },
            Operation::Store { value_changed: false }, Operation::Refresh(crate::cache::RefreshReason::Missing),
            Operation::Refresh(crate::cache::RefreshReason::Expired), Operation::Refresh(crate::cache::RefreshReason::Dependencies)];
        times.measure_batches("Batches", |batch| {
            for index in 0..MAX_TIMING_PATHS + 10 {
                let name: &'static str = Box::leak(format!("Batch {index}").into_boxed_str());
                batch.measure(name, || ());
                for operation in operations {
                    batch.record_memory(batch.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::StellarScratch, operation));
                }
            }
            assert_eq!(batch.steps.len(), MAX_TIMING_PATHS);
            assert_eq!(batch.memory_batch_events, MAX_TRACE_EVENTS);
            assert_eq!(batch.records.iter().map(|record| record.memory_events.len()).sum::<usize>(), MAX_TRACE_EVENTS);
            assert!(batch.records.iter().map(|record| record.memory_events.capacity()).sum::<usize>() <= MAX_TRACE_EVENTS * 2);
        });
        let trace = times.trace().unwrap();
        assert_eq!(trace.event_count, MAX_TRACE_EVENTS);
        assert!(trace.steps.iter().map(|step| step.memory_omitted).sum::<usize>() > 0);
        assert!(times.registry_omitted > 0);
    }

    #[test]
    fn inventories_are_bounded_per_segment_and_report_moves_as_transfers() {
        use super::super::memory::{BufferId, MemoryEvent, Operation};
        let mut times = StepTimes::default();
        times.enable_memory_run(true);
        times.capture_memory(|_| crate::cache::InventorySnapshot { label: "test", simulated_tt: None, rows: Vec::new(), omitted_nodes: 0, root_inline: 0, collector_retained_bytes: Some(0), collector_temporary_bytes: Some(0), capture_seconds: 0.0 });
        times.capture_memory(|_| panic!("inventory cap must stay lazy"));
        assert_eq!(times.trace().unwrap().bounds.omitted_inventories, 1);
        times.begin_memory_frame();
        times.measure("Move", || ());
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Move, None, None, Some(2), Some(16)));
        times.capture_memory(|_| crate::cache::InventorySnapshot { label: "test", simulated_tt: None, rows: Vec::new(), omitted_nodes: 0, root_inline: 0, collector_retained_bytes: Some(0), collector_temporary_bytes: Some(0), capture_seconds: 0.0 });
        times.complete_memory_frame(0.0);
        assert_eq!(times.memory_inventories().count(), 2);
        let mut output = Vec::new();
        times.write_memory_run_report(&mut output).unwrap();
        let report = String::from_utf8(output).unwrap();
        assert!(report.contains("runtime=enabled"));
        assert!(report.contains("logical transferred payload=16 B"));
        assert!(!report.contains("logical copied payload"));
    }

}
