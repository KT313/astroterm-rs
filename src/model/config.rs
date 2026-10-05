//! Validated application settings, independent of CLI parsing and terminal operations.
use crate::catalog::datasets::Dataset;
use crate::astro::Observer;
use crate::model::projection::View;
use crate::model::rendering::RenderOptions;

/// Everything the application needs to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub debug_singleframe: bool,
    pub debug_memory: bool,
    pub cache: crate::cache::CacheConfig,
    /// Raster text size and spacing relative to terminal cells; ignored by native text renderers.
    pub text_scale: f64,
    pub renderer: RendererKind,
    pub graphics_protocol: GraphicsProtocol,
    pub simulation: SimulationSettings,
    /// The view to start with, and to reset to.
    pub view: View,
    pub render: RenderOptions,
    pub terminal: TerminalSettings,
    /// Frames per second.
    pub fps: u32,
    /// Star dataset to load instead of the embedded catalog.
    pub dataset: Option<Dataset>,
}

/// What is simulated: where, from when, how fast, and with which corrections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationSettings {
    pub observer: Observer,
    pub start_julian_date: f64,
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
