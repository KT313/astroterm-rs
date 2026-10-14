//! Supporting application operations. Import helpers through this module; implementation folders are private.
//! Startup opens resources, frame helpers handle controls and the panel, and diagnostics report their results.
mod startup;
mod frame;
mod diagnostics;

pub(super) use startup::{load_cities, print_bash_completions, validate_arguments, load_catalog, open_display};
pub(super) use frame::{Panel, PanelFrame, apply_frame_controls, resolve_frame_time, stop_on_quit};
pub(super) use diagnostics::{
    BenchmarkSession, open_frame_benchmark, begin_benchmark_capture, prepare_benchmark_frame,
    validate_benchmark_time, complete_benchmark_frame, finish_benchmark_report,
    finish_rendering, begin_frame_diagnostics, finish_frame_diagnostics, capture_display,
};

// Shared implementation helpers stay private to this module and its children.
use diagnostics::{finish_requested_report, report_failure};
