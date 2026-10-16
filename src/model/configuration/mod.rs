//! Validated application settings, independent of CLI parsing and terminal operations.
use crate::catalog::datasets::Dataset;
use crate::formulas::Observer;
mod view;
pub use view::{ViewCenter, ProjectionKind, View, Cell, ProjectionViewport, RenderOptions};

/// Everything the application needs to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub debug_singleframe: bool,
    pub frame_benchmark: Option<FrameBenchmarkConfig>,
    pub no_progress: bool,
    /// Enable explicitly placed table dumps; independent of timing tracing.
    pub debug_log_data: bool,
    /// Raster text size and spacing relative to terminal cells; ignored by native text renderers.
    pub text_scale: f64,
    pub graphics_protocol: GraphicsProtocol,
    pub simulation: SimulationSettings,
    /// The view to start with, and to reset to.
    pub view: View,
    pub render: RenderOptions,
    pub terminal: TerminalSettings,
    /// Frames per second.
    pub fps: u32,
    pub projection_jobs: usize,                // resolved projection worker count; zero creates no pool
    /// Star dataset to load instead of the embedded catalog.
    pub dataset: Option<Dataset>,
    pub dataset_parallel_blocks: u8,                                                     // maximum simultaneous MAST block downloads
}

/// Bounded, deterministic frame capture; the output writer remains outside the state tree.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameBenchmarkConfig {
    pub frames: u32,
    pub step_ms: i64,
    pub report: std::path::PathBuf,
}

/// What is simulated: where, from when, how fast, and with which corrections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationSettings {
    pub observer: Observer,
    pub start_time_ms: i64,
    /// Simulated days per real day.
    pub speed: f64,
    /// Lift objects by atmospheric refraction.
    pub refraction: bool,
}

/// Settings of the terminal itself, as opposed to what is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalSettings {
    /// Cell height / width; detected from the terminal when `None`.
    pub aspect_ratio: Option<f64>,
    /// Show the metadata panel in the top left corner.
    pub metadata_panel: bool,
    /// Quit on any key instead of only `q`, Esc and Ctrl-C.
    pub quit_on_any_key: bool,
    /// Show the frame step durations below the metadata.
    pub frame_times: bool,
}

/// The old renderer switch; only the old `terminal::Renderer` still reads it (gone with chunk 14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum RendererKind {
    #[default]
    Chars,
    Pixels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphicsProtocol {
    #[default]
    Auto,
    Kitty,
    Sixel,
    Iterm2,
    Halfblocks,
}
