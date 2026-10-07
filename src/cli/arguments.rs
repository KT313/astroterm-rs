//! Command line arguments, as given by the user. Options and help texts follow the original astroterm.

use std::path::PathBuf;

use clap::{ArgAction, Parser};

use crate::terminal::format_key_bindings_help;

/// View stars, planets, and more, right in your terminal! ✨🪐
#[derive(Clone, Debug, Parser)]
#[command(name = "astroterm", version, override_usage = "astroterm [OPTION]...", after_help = format_key_bindings_help())]
#[command(disable_help_flag = true, disable_version_flag = true)]
pub struct Arguments {
    /// Recompute runtime processing results every frame; keeps downloaded datasets and the on-disk catalog cache.
    #[arg(long)]
    pub disable_cache: bool,
    /// Read processing-cache policies from this TOML file instead of the user configuration directory.
    #[arg(long, value_name = "path")]
    pub cache_config: Option<std::path::PathBuf>,

    /// Renderer: characters (default) or true-color pixels. Constellations, grid, thresholds, labels,
    /// refraction and metadata apply to both; --color, --unicode and --braille affect characters only.
    #[arg(long, value_enum, default_value_t = crate::model::RendererKind::Chars)]
    pub renderer: crate::model::RendererKind,

    /// Pixel protocol (normally detected). Force one to test terminal support; halfblocks needs no graphics protocol.
    #[arg(long, value_enum, default_value_t = crate::model::GraphicsProtocol::Auto)]
    pub graphics_protocol: crate::model::GraphicsProtocol,

    /// Raster text scale relative to terminal cells [0.25–4], for Sixel/Kitty/iTerm2 only. 1 restores the
    /// terminal-cell-sized layout; characters and native half-block text are unaffected.
    #[arg(long, default_value_t = 0.85, value_name = "factor", allow_negative_numbers = true)]
    pub text_scale: f64,

    /// Observer latitude [-90°, 90°] (default: 0.0)
    #[arg(
        short = 'a',
        long,
        value_name = "degrees",
        allow_negative_numbers = true,
        default_value_t = 0.0,
        hide_default_value = true
    )]
    pub latitude: f64,

    /// Observer longitude [-180°, 180°] (default: 0.0)
    #[arg(
        short = 'o',
        long,
        value_name = "degrees",
        allow_negative_numbers = true,
        default_value_t = 0.0,
        hide_default_value = true
    )]
    pub longitude: f64,

    /// Observation datetime in UTC/UT, proleptic Gregorian calendar. Astronomical years: 0 = 1 BC; use a sign
    /// outside 0000–9999, e.g. -7974-01-01T00:00:00 or +12026-01-01T00:00:00
    #[arg(short = 'd', long, value_name = "yyyy-mm-ddThh:mm:ss", allow_hyphen_values = true)]
    pub datetime: Option<String>,

    /// Only render stars brighter than this magnitude (default: 5.0)
    #[arg(
        short = 't',
        long,
        value_name = "float",
        allow_negative_numbers = true,
        default_value_t = 5.0,
        hide_default_value = true
    )]
    pub threshold: f32,

    /// Label stars brighter than this magnitude (default: 0.25)
    #[arg(
        short = 'l',
        long = "label-thresh",
        value_name = "float",
        allow_negative_numbers = true,
        default_value_t = 0.25,
        hide_default_value = true
    )]
    pub label_threshold: f32,

    /// Frames per second (default: 24 for characters, 12 for pixels)
    #[arg(short = 'f', long, value_name = "int", allow_negative_numbers = true)]
    pub fps: Option<i64>,

    /// Animation speed multiplier (default: 1.0)
    #[arg(
        short = 's',
        long,
        value_name = "float",
        allow_negative_numbers = true,
        default_value_t = 1.0,
        hide_default_value = true
    )]
    pub speed: f64,

    /// Enable terminal colors
    #[arg(short = 'c', long)]
    pub color: bool,

    /// Draw constellation stick figures. Note: a constellation is only drawn if all stars in the figure are over the
    /// threshold
    #[arg(short = 'C', long)]
    pub constellations: bool,

    /// Draw an azimuthal grid
    #[arg(short = 'g', long)]
    pub grid: bool,

    /// Use unicode characters
    #[arg(short = 'u', long)]
    pub unicode: bool,

    /// Use braille characters for constellation lines (requires Unicode)
    #[arg(short = 'b', long)]
    pub braille: bool,

    /// Quit on any keypress (default is to quit on 'q' or 'ESC' only)
    #[arg(short = 'q', long = "quit-on-any")]
    pub quit_on_any: bool,

    /// Display metadata
    #[arg(short = 'm', long)]
    pub metadata: bool,

    /// Override the calculated terminal cell aspect ratio. Use this if your projection is not 'square.' A value around
    /// 2.0 works well for most cases
    #[arg(
        short = 'r',
        long = "aspect-ratio",
        value_name = "float",
        allow_negative_numbers = true
    )]
    pub aspect_ratio: Option<f64>,

    /// Apply atmospheric refraction: objects near the horizon appear up to about 0.5° higher, as in the real sky
    #[arg(short = 'R', long)]
    pub refraction: bool,

    /// Load athyg (downloads about 200 MB on first use), or an AT-HYG CSV file (.csv or .csv.gz), instead of
    /// the built-in Yale Bright Star Catalog. Existing files and values containing a path separator are paths.
    /// Constellation figures are matched by HR number
    #[arg(long, value_name = "name|path")]
    pub dataset: Option<PathBuf>,

    /// Don't name extra stars when zooming in. By default, if fewer than 5 objects in view have names, the brightest
    /// visible stars are named too (with their catalog number if they have no proper name)
    #[arg(long = "disable-dynamic-names")]
    pub disable_dynamic_names: bool,

    /// Show how long each step of the per-frame calculation and rendering takes (smoothed), below the metadata.
    /// Turns on --metadata
    #[arg(long = "debug-frametimes")]
    pub debug_frametimes: bool,

    /// Present one frame, restore the terminal, then print ordered pipeline timings and data counts
    #[arg(long)]
    pub debug_singleframe: bool,

    /// Append Markdown table dumps in tmp/ at startup, after preparation, and after each projection and rendering. Available in every build
    #[arg(long)]
    pub debug_log_data: bool,

    /// Report bounded memory inventories and instrumented operations after quitting; combine with --debug-singleframe for one frame.
    /// Requires: cargo build --features memory-diagnostics --bin astroterm. Does not enable the on-screen timing panel
    #[arg(long)]
    pub debug_memory: bool,

    /// Print this help message
    #[arg(short = 'h', long, action = ArgAction::Help)]
    help: Option<bool>,

    /// Print bash completions
    #[arg(short = 'B', long = "bash-completions")]
    pub bash_completions: bool,

    /// Use the latitude and longitude of the provided city. If the name contains multiple words, enclose the name in
    /// single or double quotes. For a list of available cities, see:
    /// <https://github.com/da-luce/astroterm/blob/main/data/cities.csv>
    #[arg(short = 'i', long, value_name = "city_name")]
    pub city: Option<String>,

    /// Show the sky in the direction you are facing instead of overhead, with the horizon across the middle. Give a
    /// compass azimuth in degrees [0°, 360°] (0 is North, 90 is East) or a compass direction such as N or NNW. The
    /// --grid option has no effect in this view
    #[arg(short = 'F', long, value_name = "azimuth")]
    pub facing: Option<String>,

    /// Tilt the facing view up (positive) or down (negative) from the horizon [-90°, 90°] (default: 0.0). Only
    /// supported with --facing. --facing S --tilt 90 shows the same view as leaving out --facing
    #[arg(short = 'T', long, value_name = "degrees", allow_negative_numbers = true)]
    pub tilt: Option<f64>,

    /// Field of view of the rendered circle (0°, 359°] (default: 180.0), up to 360 with --equidistant. Smaller values
    /// zoom in on the center of the view, which is the zenith when --facing is not used. Larger values also show the
    /// sky behind it, increasingly distorted
    #[arg(short = 'z', long, value_name = "degrees", allow_negative_numbers = true)]
    pub fov: Option<f64>,

    /// Use an azimuthal equidistant projection instead of stereographic. Distances from the center are proportional
    /// to the angle, so there is less distortion near the edge, and --fov can go up to 360 (the point directly behind
    /// the view becomes the edge of the circle)
    #[arg(short = 'e', long)]
    pub equidistant: bool,

    /// Display version info and exit
    #[arg(short = 'v', long, action = ArgAction::Version)]
    version: Option<bool>,
}
