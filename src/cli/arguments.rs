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

    /// Pixel protocol (normally detected). Force one to test terminal support; halfblocks needs no graphics protocol.
    #[arg(long, value_enum, default_value_t = crate::model::GraphicsProtocol::Auto)]
    pub graphics_protocol: crate::model::GraphicsProtocol,

    /// Raster text scale relative to terminal cells [0.25–4], for Sixel/Kitty/iTerm2 only. 1 restores the
    /// terminal-cell-sized layout; characters and native half-block text are unaffected.
    #[arg(long, default_value_t = crate::constants::DEFAULT_PIXEL_TEXT_SCALE, value_name = "factor", allow_negative_numbers = true)]
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
        default_value_t = crate::constants::DEFAULT_MAGNITUDE_THRESHOLD,
        hide_default_value = true
    )]
    pub threshold: f32,

    /// Frames per second (default: 12)
    #[arg(short = 'f', long, value_name = "int", allow_negative_numbers = true)]
    pub fps: Option<i64>,

    /// Animation speed multiplier (default: 1.0)
    #[arg(
        short = 's',
        long,
        value_name = "float",
        allow_negative_numbers = true,
        default_value_t = crate::constants::DEFAULT_SIMULATION_SPEED,
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

    /// Load athyg (~200 MB), gaia-gdr3 (requires --dataset-files or --dataset-stars), or an AT-HYG CSV (.csv or .csv.gz), instead of
    /// the built-in Yale Bright Star Catalog. Existing files and values containing a path separator are paths.
    /// Constellation figures are matched by HR number for AT-HYG; Gaia subsets have no figures
    #[arg(long, value_name = "name|path")]
    pub dataset: Option<PathBuf>,

    /// Number of Gaia DR3 files to load, in fixed sky order (0 means no catalog stars). Only for gaia-gdr3
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(0..=i64::from(crate::catalog::datasets::GAIA_DR3_FILE_COUNT)))]
    pub dataset_files: Option<u16>,

    /// Select the first N physical Gaia DR3 rows from MAST Parquet (whole blocks may be downloaded).
    /// Missing measurements and duplicate copies reduce the loaded count. Only for gaia-gdr3; 0 means no stars
    #[arg(long, value_name = "N", conflicts_with = "dataset_files")]
    pub dataset_stars: Option<u32>,

    /// Concurrent MAST blocks [1–16] (default: 4); all 13 column requests each, reduced by the memory budget. Requires --dataset-stars.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=i64::from(crate::constants::MAST_MAX_PARALLEL_BLOCKS)))]
    pub dataset_parallel_blocks: Option<u8>,

    /// Hide dataset progress updates; warnings, errors and final loading notices remain visible
    #[arg(long)]
    pub no_progress: bool,

    /// Hide star labels. By default, label the five brightest visible stars, using catalog identifiers when
    /// proper names are unavailable. Sun, planet and Moon labels are independent
    #[arg(long = "disable-dynamic-names")]
    pub disable_dynamic_names: bool,

    /// Show how long each step of the per-frame calculation and rendering takes (smoothed), below the metadata.
    /// Turns on --metadata
    #[arg(long = "debug-frametimes")]
    pub debug_frametimes: bool,

    /// Present one frame, restore the terminal, then print ordered pipeline timings and data counts
    #[arg(long)]
    pub debug_singleframe: bool,

    /// Capture N completed frames (including reused displays) to --debug-frame-report; no interactive controls
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..), requires = "debug_frame_report",
        conflicts_with_all = ["debug_singleframe", "debug_log_data", "speed"])]
    pub debug_frames: Option<u32>,

    /// New JSONL timing file for --debug-frames; existing files are never overwritten
    #[arg(long, value_name = "FILE", requires = "debug_frames")]
    pub debug_frame_report: Option<std::path::PathBuf>,

    /// Simulated seconds per captured frame (default: 0, paused); negative values run backwards
    #[arg(long, value_name = "SECONDS", allow_negative_numbers = true, requires = "debug_frames")]
    pub debug_frame_step_seconds: Option<f64>,

    /// Append Markdown table dumps in tmp/ at startup, after preparation, and after each projection and rendering. Available in every build
    #[arg(long)]
    pub debug_log_data: bool,

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
