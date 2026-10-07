//! Supporting application operations. Import helpers through this module; implementation folders are private.
//! Startup opens resources, frame helpers run individual stages, and diagnostics describe/report their results.
mod startup;
mod frame;
mod diagnostics;

pub(super) use startup::{load_cities, print_bash_completions, validate_arguments, load_catalog, prepare_terminal};
pub(super) use frame::{
    apply_frame_controls, resolve_frame_time, simulate_frame, observe_frame, project_frame,
    render_projected_frame, stop_on_quit,
};
pub(super) use diagnostics::{
    start_step_times, configure_memory_reporting, log_pipeline_data_if_requested, finish_rendering,
    capture_failed_frame_memory, begin_frame_diagnostics, finish_frame_diagnostics, capture_memory,
};

// Shared implementation helpers stay private to this module and its children.
use diagnostics::{
    finish_requested_report, report_failure, describe_observer_geometry, describe_light_time_sampling,
    record_projected_memory,
};
