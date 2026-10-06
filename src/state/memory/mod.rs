//! Capture the ownership tree once, then format its saved inventory without touching live buffers.
mod collection;
mod report;

pub use collection::{InventoryCollector, KnownPayload, MAX_ROWS, MAX_DEPTH, MAX_CHILDREN, DETAIL_CHILDREN, MAX_VISITS, sum_known_payload};
pub use report::write_inventory;
use collection::sum_payload;
use std::mem::size_of;
use crate::cache::{BufferSink, InventorySnapshot, Owner, ReportBuffers, report_field};
use crate::{model::{Config, SkyCatalog}, state::{ApplicationState, RunState}, timing::StepTimes};

pub fn collect_inventory<T: ReportBuffers>(label: &'static str, value: &T) -> InventorySnapshot {
    let mut collector = InventoryCollector::new(label, None);
    report_field(&mut collector, "root", value);
    collector.finish()
}

/// Capture all active-run application storage; only the scoped terminal writer remains external.
#[allow(clippy::too_many_arguments)]
pub fn capture_run_inventory<R: ReportBuffers>(config: &Config, catalog: &std::sync::Arc<SkyCatalog>, run: &RunState, renderer: &R, times: &StepTimes, label: &'static str, tt: Option<f64>) -> InventorySnapshot {
    let mut collector = InventoryCollector::new(label, tt);                  // start a bounded inventory for this simulated time
    collector.enter("state", size_of::<ApplicationState>());
    report_application_buffers(&mut collector, config, catalog, run, renderer); // list the catalog, active buffers and scoped terminal resources
    report_diagnostic_coverage(&mut collector, times);                       // include retained diagnostics and identify unmeasured storage
    collector.leave();
    collector.finish()                                                     // save the inventory and the collector's own storage sizes
}

fn report_application_buffers<R: ReportBuffers>(collector: &mut InventoryCollector, config: &Config, catalog: &std::sync::Arc<SkyCatalog>, run: &RunState, renderer: &R) {
    report_field(collector, "config", config);
    report_field(collector, "catalog", catalog);
    report_field(collector, "run", run);
    crate::cache::report_external(collector, "terminal_guard_outside_root", renderer);
}

fn report_diagnostic_coverage(collector: &mut InventoryCollector, times: &StepTimes) {
    collector.set_owner(Owner::Diagnostics);
    report_field(collector, "timings", times);
    collector.set_owner(Owner::External);
    collector.unknown("startup data, bounded local requests and input controls, borrowed frame views, process-global timezone finder, allocator overhead and opaque library internals are outside complete coverage");
}
