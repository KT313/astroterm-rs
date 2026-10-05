//! The same lazy hook API is available with or without compiled-in memory diagnostics.
use astroterm::{cache::Quality, timing::{StepTimes, memory::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation}}};

#[test]
fn disabled_hooks_never_evaluate_descriptors_or_groups() {
    let mut times = StepTimes::default();
    times.with_memory(|_| panic!("disabled diagnostic group"));
    assert!(times.inspect_memory(|| panic!("disabled inspection")).is_none());
    times.record_borrow(BufferId::ObservedStars, Access::ReadOnly, || panic!("disabled borrow shape"));
    times.record_build(BufferId::ObservedStars, || panic!("disabled build shape"));
    times.record_shape(BufferId::ObservedStars, Operation::Clear, None, || panic!("disabled result shape"));
    times.record_memory(times.last_memory_step(), || panic!("disabled event"));
    times.record_candidate_decision(times.last_memory_step(), BufferId::ProjectionCandidate, BufferId::ProjectedCells, true, None);
    times.begin_memory_frame();
    times.set_memory_frame_time(1.0, 2.0);
    times.complete_memory_frame(0.0);
    times.cancel_memory_frame();
    assert!(times.trace().is_none());
}

#[test]
fn exact_element_counts_remain_exact_when_bytes_are_unknown() {
    for count in [0, 17] {
        let event = MemoryEvent::operation(BufferId::StellarSamples, Operation::Store { value_changed: true }, None, None, Some(count), None);
        assert!(matches!(event, MemoryEvent::Operation { quality: Quality::ExactPayload, elements: Some(n), logical_bytes: None, .. } if n == count));
    }
    let unknown = MemoryEvent::operation(BufferId::StellarSamples, Operation::Compare, None, None, None, None);
    assert!(matches!(unknown, MemoryEvent::Operation { quality: Quality::Unknown, .. }));
    let shape = BufferShape::unknown(IndexDomain::Catalog);
    let partial = MemoryEvent::operation(BufferId::StellarSamples, Operation::Build, Some(shape), None, Some(5), None);
    assert!(matches!(partial, MemoryEvent::Operation { quality: Quality::Unknown, .. })); // an explicit unknown shape stays unknown
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn full_step_table_still_accepts_details_for_retained_steps() {
    use astroterm::timing::run::MAX_TRACE_STEPS;
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    for _ in 0..MAX_TRACE_STEPS { times.measure("Kept", || ()); }
    times.describe("Kept", || "last retained row still has room for details".into());
    assert_eq!(times.trace().unwrap().steps.last().unwrap().details, ["last retained row still has room for details"]);
    times.measure("Dropped", || ());
    times.describe("Dropped", || panic!("omitted target must not format details"));
    assert_eq!(times.trace().unwrap().bounds.omitted_details, 1);
    times.describe("Kept", || "kept target still writable after overflow".into());
    assert_eq!(times.trace().unwrap().steps.last().unwrap().details.len(), 2);
}

#[test]
fn ordinary_trace_keeps_processing_but_never_enables_memory_work() {
    let mut times = StepTimes::with_trace(true);
    let mut values = vec![2_u64, 3];
    let result = times.measure_memory_scope("Memory-only scope", |times| {
        times.with_memory(|_| panic!("ordinary tracing must not enable the diagnostic group"));
        let before = times.inspect_memory(|| panic!("ordinary tracing must not inspect memory"));
        times.measure("Actual work", || values.push(5));
        times.record_shape(BufferId::MotionSamples, Operation::Append, before, || panic!("inactive shape"));
        values.iter().sum::<u64>()
    });
    assert_eq!(result, 10);
    assert_eq!(values, [2, 3, 5]);
    assert_eq!(times.steps().iter().map(|step| step.name).collect::<Vec<_>>(), ["Actual work"]);
    assert_eq!(times.trace().unwrap().steps.iter().map(|step| step.name).collect::<Vec<_>>(), ["Actual work"]);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn recorded_shapes_remain_values_after_the_source_is_cleared_and_dropped() {
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    let original_capacity;
    {
        let mut values = Vec::<u64>::with_capacity(19);
        values.extend([4, 5, 6]);
        original_capacity = values.capacity();
        times.measure("Use source", || ());
        times.record_borrow(BufferId::MotionSamples, Access::ReadOnly, || BufferShape::vector(&values, IndexDomain::Working));
        values.clear();
        values.shrink_to_fit();
    }
    let MemoryEvent::Borrow { shape, .. } = times.trace().unwrap().steps[0].memory_events[0].event else { panic!("borrow descriptor"); };
    assert_eq!(shape.len, Some(3));
    assert_eq!(shape.capacity, Some(original_capacity));
    assert_eq!(shape.logical_bytes(), Some(24));
    assert_eq!(shape.domain, IndexDomain::Working);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn aggregate_overflow_reports_unknown_instead_of_wrapping_or_losing_other_counts() {
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    times.measure_batches("Batches", |batch| {
        for count in [usize::MAX, 1] {
            batch.measure("Pass", || ());
            batch.record_memory(batch.last_memory_step(), || MemoryEvent::operation(
                BufferId::MotionSamples, Operation::Copy, None, None, Some(count), Some(count)));
            batch.record_memory(batch.last_memory_step(), || MemoryEvent::operation(
                BufferId::StellarScratch, Operation::Write, None, None, Some(1), None));
        }
    });
    let events = &times.trace().unwrap().steps[1].memory_events;
    assert_eq!(events[0].calls, 2);
    assert_eq!(events[0].total_elements, None);
    assert_eq!(events[0].total_logical_bytes, None);
    assert_eq!(events[1].total_elements, Some(2));
    assert_eq!(events[1].total_logical_bytes, None);
    let mut output = Vec::new();
    times.trace().unwrap().write_report(&mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("logical elements=unknown; logical copied payload=unknown"));
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn nested_memory_scopes_propagate_errors_without_polluting_the_completed_frame() {
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    times.begin_memory_frame();
    times.set_memory_frame_time(10.0, 11.0);
    times.measure("Completed work", || ());
    times.complete_memory_frame(0.1);
    let completed = times.memory_run().unwrap().latest.clone();
    let totals = times.memory_run().unwrap().aggregates.clone();

    times.begin_memory_frame();
    times.set_memory_frame_time(20.0, 21.0);
    let result: Result<(), &str> = times.measure_steps("Failed parent", |times| {
        times.measure_memory_scope("Memory scope", |times| {
            times.measure_with_memory("Failing leaf", |times| {
                times.record_memory(times.active_memory_step(), || MemoryEvent::unknown_operation(BufferId::UploadBytes, Operation::Write));
                Err("simulated write failure")
            })?;
            Ok(())
        })
    });
    assert_eq!(result, Err("simulated write failure"));
    assert!(times.active_memory_step().is_none());
    times.measure("After failure", || ());
    times.record_unknown(BufferId::UploadBytes, Operation::Release);
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps.iter().map(|step| (step.name, step.depth)).collect::<Vec<_>>(),
        [("Failed parent", 0), ("Memory scope", 1), ("Failing leaf", 2), ("After failure", 0)]);
    assert_eq!(trace.steps[2].memory_events.len(), 1);
    assert_eq!(trace.steps[3].memory_events.len(), 1);
    assert!(trace.steps[0].seconds >= trace.steps[1].seconds);
    assert!(trace.steps[1].seconds >= trace.steps[2].seconds);
    assert!(trace.steps[2].seconds >= trace.steps[2].direct_diagnostic_seconds);
    assert_eq!(times.memory_run().unwrap().latest, completed);
    assert_eq!(times.memory_run().unwrap().aggregates, totals);
    assert_eq!(times.memory_run().unwrap().completed_frames, 1);
    assert_eq!(times.memory_run().unwrap().current_time, Some((20.0, 21.0)));
    let mut output = Vec::new();
    times.write_memory_run_report(&mut output).unwrap();
    let report = String::from_utf8(output).unwrap();
    assert!(report.contains("completed frames=1"));
    assert!(report.contains("Latest completed frame: UTC=Some(10.0)"));
    assert!(report.contains("Incomplete frame: simulated time=Some((20.0, 21.0))"));
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn omitted_repeated_invocation_does_not_describe_its_predecessor() {
    use astroterm::timing::run::MAX_TRACE_STEPS;
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    times.measure("Retained different name", || ());
    for _ in 1..MAX_TRACE_STEPS { times.measure("Same", || ()); }
    times.describe("Same", || "last retained invocation".into());
    times.measure("Same", || ());
    times.describe("Same", || panic!("the omitted call must not describe its retained predecessor"));
    times.describe("Retained different name", || "still writable".into());
    assert_eq!(times.trace().unwrap().steps.last().unwrap().details, ["last retained invocation"]);
    assert_eq!(times.trace().unwrap().steps[0].details, ["still writable"]);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn suppressed_descendants_do_not_supersede_a_retained_sibling_with_the_same_name() {
    use astroterm::timing::run::MAX_TRACE_STEPS;
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    times.measure_steps("Retained parent", |times| {
        times.measure("Same", || ());
        for _ in 2..MAX_TRACE_STEPS { times.measure("Filler", || ()); }
        times.measure_steps("Omitted parent", |times| {
            times.measure("Same", || ());
            times.describe("Same", || panic!("suppressed child must not format a description"));
        });
        times.describe("Same", || "the earlier child still belongs to the retained parent".into());
    });
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps[1].details, ["the earlier child still belongs to the retained parent"]);
    assert_eq!(trace.bounds.omitted_steps, 2);
    assert_eq!(trace.bounds.omitted_details, 1);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn activation_truncation_preserves_old_details_without_retargeting_the_truncated_call() {
    use astroterm::timing::run::MAX_TRACE_STEPS;
    let mut times = StepTimes::with_trace(true);
    times.measure("Retained different name", || ());
    for _ in 1..MAX_TRACE_STEPS { times.measure("Same", || ()); }
    times.describe("Same", || "old retained detail".into());
    times.measure("Same", || ());
    times.describe("Same", || "new truncated detail".into());
    times.enable_memory_run(true);
    times.describe("Same", || panic!("truncation must not retarget the latest invocation"));
    times.describe("Retained different name", || "still writable".into());
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps.last().unwrap().details, ["old retained detail"]);
    assert_eq!(trace.steps[0].details, ["still writable"]);
    assert_eq!(trace.bounds.omitted_steps, 1);
    assert_eq!(trace.bounds.omitted_details, 1);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn activation_truncation_does_not_confuse_children_of_dropped_parents_with_roots() {
    use astroterm::timing::run::MAX_TRACE_STEPS;
    let mut times = StepTimes::with_trace(true);
    times.measure("Same", || ());
    for _ in 1..MAX_TRACE_STEPS { times.measure("Filler", || ()); }
    times.measure_steps("Truncated parent", |times| times.measure("Same", || ()));
    times.enable_memory_run(true);
    times.describe("Same", || "retained root is still the latest root with this name".into());
    let trace = times.trace().unwrap();
    assert_eq!(trace.steps[0].details, ["retained root is still the latest root with this name"]);
    assert_eq!(trace.bounds.omitted_steps, 2);
    assert_eq!(trace.bounds.omitted_details, 0);
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn candidate_decisions_attach_to_explicit_steps_and_tolerate_unknown_reasons() {
    use astroterm::cache::RefreshReason;
    let mut times = StepTimes::default();
    times.enable_memory_run(true);
    times.measure_steps("Geometry", |times| {
        let active = times.active_memory_step();
        times.measure("Interleaved child", || ()); // the active target differs from the last completed target
        times.record_candidate_decision(active, BufferId::ProjectionBodyCandidate, BufferId::ProjectedBodies, true, Some(RefreshReason::Dependencies));
    });
    times.measure("No reason supplied", || ());
    times.record_candidate_decision(times.last_memory_step(), BufferId::ProjectionCandidate, BufferId::ProjectedCells, true, None);
    times.measure("Reused", || ());
    times.record_candidate_decision(times.last_memory_step(), BufferId::PixelCandidate, BufferId::PixelScene, false, None);

    let steps = &times.trace().unwrap().steps;
    assert_eq!(steps[0].memory_events.len(), 2);
    assert!(steps[1].memory_events.is_empty());
    assert!(matches!(steps[0].memory_events[0].event, MemoryEvent::Operation { buffer: BufferId::ProjectionBodyCandidate, operation: Operation::Compare, .. }));
    assert!(matches!(steps[0].memory_events[1].event, MemoryEvent::Operation { buffer: BufferId::ProjectedBodies, operation: Operation::Refresh(RefreshReason::Dependencies), .. }));
    assert_eq!(steps[2].memory_events.len(), 1);
    assert!(matches!(steps[2].memory_events[0].event, MemoryEvent::Operation { buffer: BufferId::ProjectedCells, operation: Operation::RefreshUnknown, quality: Quality::Unknown, .. }));
    assert!(matches!(steps[3].memory_events[0].event, MemoryEvent::Operation { operation: Operation::Compare, .. }));
    assert!(matches!(steps[3].memory_events[1].event, MemoryEvent::Operation { operation: Operation::Reuse, .. }));
    let mut report = Vec::new();
    times.trace().unwrap().write_report(&mut report).unwrap();
    assert!(String::from_utf8(report).unwrap().contains("refresh required (reason unknown)"));
}
