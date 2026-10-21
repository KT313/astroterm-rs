//! Supporting application operations. Import helpers through this module; implementation folders are private.
//! Startup opens resources, frame helpers handle controls and the panel, and diagnostics report their results.
//! Before all of them, the program may restart itself once with huge-page memory (`huge_pages`).
mod huge_pages;
mod startup;
mod frame;
mod diagnostics;

pub(super) use huge_pages::restart_with_huge_pages_if_allowed;

pub(super) use startup::{load_cities, print_bash_completions, validate_arguments, load_catalog, open_display};
pub(super) use frame::{Panel, draw_text_overlay, handle_input};
pub(super) use diagnostics::{
    BenchmarkSession, open_frame_benchmark, begin_benchmark_capture, validate_benchmark_input, set_benchmark_frame_time,
    complete_benchmark_frame, finish_benchmark_report,
    finish_rendering, begin_frame_diagnostics, finish_frame_diagnostics, capture_display,
};

// Shared implementation helpers stay private to this module and its children.
use diagnostics::{finish_requested_report, report_failure};
