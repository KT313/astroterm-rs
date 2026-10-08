//! Format bounded run evidence after terminal cleanup; the inventory printer belongs to state.
use crate::constants::{MAX_AGGREGATE_PATHS, MAX_DETAIL_BYTES, MAX_TIMING_PATHS, MAX_TRACE_DEPTH, MAX_TRACE_DETAILS, MAX_TRACE_EVENTS, MAX_TRACE_INVENTORIES, MAX_TRACE_STEPS, MAX_TRACE_TEXT_BYTES};
use super::{MemoryRun, PipelineTrace};
use std::io::{self, Write};

pub(super) fn write_run_header(output: &mut impl Write, run: &MemoryRun, registry_omitted: u64) -> io::Result<()> {
    writeln!(output, "astroterm --debug-memory: run report")?;
    writeln!(output, "Build feature=memory-diagnostics; runtime=enabled; completed frames={}; cache enabled={}", run.completed_frames, run.cache_enabled)?;
    writeln!(output, "Retained: startup + latest completed frame + current incomplete frame. Sums/counts below cover bounded step evidence from completed frames only; parent sums include children. No partial frame is counted as presented.")?;
    writeln!(output, "Limits per segment: steps={MAX_TRACE_STEPS}; depth={MAX_TRACE_DEPTH}; events={MAX_TRACE_EVENTS} (per step={}); detail strings={MAX_TRACE_DETAILS}; detail bytes={MAX_TRACE_TEXT_BYTES}; single detail bytes={MAX_DETAIL_BYTES}; inventories={MAX_TRACE_INVENTORIES}; timing paths={MAX_TIMING_PATHS}; aggregate paths={MAX_AGGREGATE_PATHS}", crate::constants::MAX_MEMORY_EVENTS_PER_STEP)?;
    writeln!(output, "Omissions across completed frames: steps={}; event observations={}; details={}; inventories={}; discarded detail bytes={}; aggregate step rows={}; timing registrations across run={}", run.completed_omitted_steps, run.completed_omitted_events, run.completed_omitted_details, run.completed_omitted_inventories, run.completed_truncated_text_bytes, run.omitted_aggregate_steps, registry_omitted)?;
    Ok(())
}

pub(super) fn write_completed_totals(output: &mut impl Write, run: &MemoryRun) -> io::Result<()> {
    writeln!(output, "Completed-frame step totals (elapsed sums, not smoothed averages):")?;
    for aggregate in &run.aggregates {
        writeln!(output, "  {}: invocations={}; elapsed sum={:.3} ms", aggregate.path.join(" / "), aggregate.invocations, aggregate.seconds * 1000.0)?;
    }
    Ok(())
}

pub(super) fn write_retained_segments(output: &mut impl Write, run: &MemoryRun, pending: &Option<PipelineTrace>) -> io::Result<()> {
    writeln!(output, "Trace descriptor/aggregation time is charged to diagnostic totals. Frame elapsed stops before final inventory/retention work. Report formatting runs after cleanup; temporary callback strings and opaque allocations are outside retained-history byte caps.")?;
    if let Some(startup) = &run.startup { writeln!(output, "Startup:")?; startup.write_segment(output)?; }
    if let Some(frame) = &run.latest {
        writeln!(output, "Latest completed frame: UTC={:?}; TT={:?}; elapsed={:.3} ms", frame.utc, frame.tt, frame.elapsed_seconds * 1000.0)?;
        frame.trace.write_segment(output)?;
    }
    if run.frame_active {
        writeln!(output, "Incomplete frame: simulated time={:?}; not counted as presented", run.current_time)?;
        if let Some(trace) = pending { trace.write_segment(output)?; }
    } else if run.startup.is_none() && let Some(trace) = &pending { writeln!(output, "Startup:")?; trace.write_segment(output)?; }
    Ok(())
}
