//! Typed events belong to exact calls, are lazy when disabled, and aggregate only explicitly batched work.
use astroterm::cache::{Cache, Quality};
use astroterm::timing::{StepTimes, Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};

#[path = "memory_trace/observation.rs"] mod observation;
#[path = "memory_trace/projection.rs"] mod projection;
#[path = "memory_trace/rendering.rs"] mod rendering;
#[path = "memory_trace/run_pipeline.rs"] mod run_pipeline;

fn enabled() -> StepTimes {
    let mut times = StepTimes::with_trace(true);
    times.enable_memory_events(true);
    times
}

#[test]
fn disabled_memory_hooks_do_not_evaluate_arguments_even_when_normal_tracing_is_on() {
    for trace in [false, true] {
        let mut times = StepTimes::with_trace(trace);
        times.measure("Step", || ());
        assert!(times.inspect_memory(|| panic!("disabled descriptor")).is_none());
        times.record_memory(times.last_memory_step(), || panic!("disabled event"));
        if let Some(trace) = times.trace() { assert!(trace.steps.iter().all(|step| step.memory_events.is_empty())); }
    }
    let mut missing_trace = StepTimes::default();
    missing_trace.enable_memory_events(true);
    assert!(!missing_trace.memory_events_enabled());
}

#[test]
fn repeated_and_nested_calls_attach_to_exact_invocations() {
    let mut times = enabled();
    times.measure_steps("Parent", |times| {
        let parent = times.active_memory_step();
        times.measure("Same", || ());
        let first = times.last_memory_step();
        times.measure("Same", || ());
        let second = times.last_memory_step();
        assert_ne!(first, second);
        times.record_memory(first, || MemoryEvent::unknown_operation(BufferId::ProjectedCells, Operation::Clear));
        times.record_memory(second, || MemoryEvent::unknown_operation(BufferId::DrawOrder, Operation::Reuse));
        times.record_memory(parent, || MemoryEvent::borrow(BufferId::ObservedStars, Access::ReadOnly, BufferShape::slice(&[1_u8, 2], IndexDomain::Observed)));
    });
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps.len(), 3);
    assert_eq!(trace.steps.iter().map(|s| s.depth).collect::<Vec<_>>(), [0, 1, 1]);
    assert!(matches!(trace.steps[1].memory_events[0].event, MemoryEvent::Operation { operation: Operation::Clear, .. }));
    assert!(matches!(trace.steps[2].memory_events[0].event, MemoryEvent::Operation { operation: Operation::Reuse, .. }));
    assert!(trace.steps[0].direct_diagnostic_seconds > 0.0);
    assert_eq!(trace.steps[1].direct_diagnostic_seconds, 0.0); // event construction ran after the leaf timer
    assert!(trace.steps[0].seconds >= trace.steps[1].seconds + trace.steps[2].seconds + trace.steps[0].direct_diagnostic_seconds);
}

#[test]
fn batch_memory_events_are_bounded_aggregates_with_unknown_counts_preserved() {
    let mut times = enabled();
    times.measure_batches("Batches", |batch| {
        for n in 1..=1000 {
            batch.measure("Pass", || ());
            batch.record_memory(batch.last_memory_step(), || MemoryEvent::operation(BufferId::MotionSamples, Operation::Append, None, None, Some(n), Some(n * 8)));
            batch.record_memory(batch.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::MotionSamples, Operation::Compare));
        }
    });
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps.len(), 2);
    let pass = &trace.steps[1];
    assert!(pass.memory_aggregated);
    assert_eq!(pass.memory_events.len(), 2);
    assert_eq!(pass.memory_events[0].calls, 1000);
    assert_eq!(pass.memory_events[0].total_elements, Some(500500));
    assert_eq!(pass.memory_events[0].total_logical_bytes, Some(4004000));
    assert_eq!(pass.memory_events[1].total_elements, None);
    assert_eq!(pass.memory_events[1].total_logical_bytes, None);
    assert!(trace.steps[0].seconds >= pass.seconds + trace.steps[0].direct_diagnostic_seconds);
}

#[test]
fn clear_and_copy_descriptors_mean_logical_payload_not_allocator_or_traffic_claims() {
    let mut times = enabled();
    let mut values = Vec::with_capacity(20);
    values.extend([1_u64, 2, 3]);
    let before = times.inspect_memory(|| BufferShape::vector(&values, IndexDomain::Working));
    times.measure("Clear", || values.clear());
    times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::StellarScratch, Operation::Clear, before, Some(BufferShape::vector(&values, IndexDomain::Working)), Some(3), None));
    let event = times.trace().unwrap().steps[0].memory_events[0].event;
    let MemoryEvent::Operation { before: Some(before), after: Some(after), logical_bytes, quality, .. } = event else { panic!("clear descriptor"); };
    assert_eq!(before.capacity, after.capacity);
    assert_eq!(after.len, Some(0));
    assert_eq!(before.logical_bytes(), Some(24));
    assert_eq!(logical_bytes, None);
    assert_eq!(quality, Quality::ExactPayload);
}

#[test]
fn store_outcome_uses_existing_comparison_and_preserves_generations() {
    use std::{cell::Cell, rc::Rc};
    struct Value(u8, Rc<Cell<usize>>);
    impl PartialEq for Value {
        fn eq(&self, other: &Self) -> bool { self.1.set(self.1.get() + 1); self.0 == other.0 }
    }
    let comparisons = Rc::new(Cell::new(0));
    let mut cache = Cache::default();
    assert!(cache.store(0, 1.0, 0.0, Value(1, comparisons.clone())).value_changed);
    let generation = cache.generation;
    assert!(!cache.store(1, 2.0, 0.0, Value(1, comparisons.clone())).value_changed);
    assert_eq!(comparisons.get(), 1);
    assert_eq!(cache.generation, generation);
    assert!(cache.store(1, 3.0, 0.0, Value(2, comparisons.clone())).value_changed);
    assert_eq!(comparisons.get(), 2);
    assert_eq!(cache.generation, generation + 1);
}

#[test]
fn event_limits_and_report_units_leave_unknowns_explicit() {
    let mut times = enabled();
    times.measure("Step", || ());
    for _ in 0..astroterm::timing::MAX_MEMORY_EVENTS_PER_STEP + 2 {
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::UploadBytes, Operation::Output, None, None, Some(2048), Some(2048)));
    }
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps[0].memory_events.len(), astroterm::timing::MAX_MEMORY_EVENTS_PER_STEP);
    assert_eq!(trace.steps[0].memory_omitted, 2);
    let mut output = Vec::new();
    trace.write_report(&mut output).unwrap();
    let report = String::from_utf8(output).unwrap();
    assert!(report.contains("submitted payload=2.0 KiB"));
    assert!(report.contains("not display completion"));
    assert!(report.contains("memory events omitted=2"));
    assert!(report.contains("not RAM traffic"));
    assert!(report.contains("Memory event report formatting/output:"));
}

#[test]
fn memory_only_scopes_keep_normal_timing_rows_unchanged_and_events_nested() {
    for active in [false, true] {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_events(active);
        let mut ran = false;
        let result = times.measure_memory_scope("Untimed metadata", |times| {
            ran = true;
            times.measure("Existing child", || 42);
            times.record_memory(times.last_memory_step(), || MemoryEvent::unknown_operation(BufferId::MetadataFields, Operation::Build));
            17
        });
        assert!(ran);
        assert_eq!(result, 17);
        assert_eq!(times.steps().iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(), [("Existing child", 0)]);
        let trace = times.trace().unwrap();
        if active {
            assert_eq!(trace.steps.iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(), [("Untimed metadata", 0), ("Existing child", 1)]);
            assert_eq!(trace.steps[1].memory_events.len(), 1);
        } else {
            assert_eq!(trace.steps.len(), 1);
            assert!(trace.steps[0].memory_events.is_empty());
        }
    }
}

#[test]
fn existing_leaf_context_keeps_panel_structure_and_targets_its_own_invocation() {
    for active in [false, true] {
        let mut times = StepTimes::with_trace(true);
        times.enable_memory_events(active);
        times.measure_steps("Parent", |times| {
            times.measure_with_memory("Leaf", |times| {
                times.record_memory(times.active_memory_step(), || MemoryEvent::unknown_operation(BufferId::ProjectedBodies, Operation::Build));
            });
        });
        assert_eq!(times.steps().iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(), [("Parent", 0), ("Leaf", 1)]);
        let trace = times.trace().unwrap();
        assert_eq!(trace.steps.len(), 2);
        assert!(trace.steps[0].memory_events.is_empty());
        assert_eq!(trace.steps[1].memory_events.len(), usize::from(active));
    }
}
