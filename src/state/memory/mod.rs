//! Capture the ownership tree once, then format its saved inventory without touching live buffers.
mod collection;
mod report;

pub use collection::{InventoryCollector, KnownPayload, sum_known_payload};
pub use report::write_inventory;
use collection::sum_payload;
use std::mem::size_of;
use crate::cache::{BufferSink, InventorySnapshot, Owner, ReportBuffers, report_field};
use crate::{model::{Config, SkyCatalog}, state::{ApplicationState, Caches}, timing::StepTimes};

pub fn collect_inventory<T: ReportBuffers>(label: &'static str, value: &T) -> InventorySnapshot {
    let mut collector = InventoryCollector::new(label, None);
    report_field(&mut collector, "root", value);
    collector.finish()
}

/// Capture all active-run application storage; only the scoped terminal writer remains external.
#[allow(clippy::too_many_arguments)]
pub fn capture_run_inventory<R: ReportBuffers>(config: &Config, catalog: &std::sync::Arc<SkyCatalog>, caches: &Caches, preparation: Option<&crate::model::CatalogPreparation>, renderer: &R, times: &StepTimes, label: &'static str, tt: Option<f64>) -> InventorySnapshot {
    let mut collector = InventoryCollector::new(label, tt);                  // start a bounded inventory for this simulated time
    collector.enter("state", size_of::<ApplicationState>());
    report_application_buffers(&mut collector, config, catalog, caches, preparation, renderer); // list the catalog, caches and scoped terminal resources
    report_diagnostic_coverage(&mut collector, times);                       // include retained diagnostics and identify unmeasured storage
    collector.leave();
    collector.finish()                                                     // save the inventory and the collector's own storage sizes
}

fn report_application_buffers<R: ReportBuffers>(collector: &mut InventoryCollector, config: &Config, catalog: &std::sync::Arc<SkyCatalog>, caches: &Caches, preparation: Option<&crate::model::CatalogPreparation>, renderer: &R) {
    report_field(collector, "config", config);
    collector.enter("persistent", size_of::<crate::state::Persistent>());
    report_field(collector, "catalog", catalog);
    collector.leave();
    report_field(collector, "preparation", &preparation);
    report_field(collector, "cache", caches);
    crate::cache::report_external(collector, "terminal_guard_outside_root", renderer);
}

fn report_diagnostic_coverage(collector: &mut InventoryCollector, times: &StepTimes) {
    collector.set_owner(Owner::Diagnostics);
    report_field(collector, "timings", times);
    collector.set_owner(Owner::External);
    collector.unknown("startup data, bounded local requests and input controls, borrowed frame views, process-global timezone finder, allocator overhead and opaque library internals are outside complete coverage");
}
