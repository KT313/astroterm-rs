//! Adjustable application settings, defaults and resource limits. Values are unchanged by this move.
//! Import settings from `crate::constants`; this foundation depends only on the standard library.
//! Physical/unit constants, catalog-format markers, measured validation ranges, astronomical coefficients,
//! glyph palettes and algorithm-coupled fast-path constants stay with their definitions.
//! Accuracy/grid settings require their existing qualification tests; they are not arbitrary safety margins.
//! Derived counts belong beside their input setting and must never be tuned independently.
use std::f64::consts::PI;

// Cache lifetimes (simulated seconds; configuration may shorten these)
// Other caches refresh when their inputs change, without a time limit.
pub const STELLAR_REGION_TTL_SECONDS: f64 = 10.0 * 86_400.0;                              // reuse star positions and brightness for up to 10 simulated days
pub const SOLAR_SYSTEM_GROUP_TTL_SECONDS: f64 = 0.0;                                   // reuse complete solar-system results only at the same simulated time
pub const PLANETARY_SAMPLE_TTL_SECONDS: f64 = 30.0;                                       // reuse planetary samples within 30 simulated seconds of their time
pub const LUNAR_SAMPLE_TTL_SECONDS: f64 = 12.0;                                           // reuse Moon samples within 12 simulated seconds of their time
pub const ORIENTATION_SAMPLE_TTL_SECONDS: f64 = 60.0;                                     // reuse Earth-axis orientation samples within 60 simulated seconds
pub const STELLAR_BATCH_SIZE: usize = 1024;                                               // calculate this many stars at a time in temporary working memory

// Sky regions and storage accuracy (angles are stored in radians)
pub const GRID_DEPTH: u8 = 6;                                                             // split each cube face into 64 × 64 sky regions at depth 6
pub const GRID_COARSE_DEPTH: u8 = 4;                                                      // use larger sky regions first to quickly reject areas outside the view
pub const GRID_CHILDREN_PER_COARSE_CELL: usize = 1 << (2 * (GRID_DEPTH - GRID_COARSE_DEPTH)); // number of small regions inside one large region; calculated automatically
pub const CELL_COUNT: usize = 6 << (2 * GRID_DEPTH);                                      // total ordinary sky regions across six cube faces; calculated automatically
pub const CONSTELLATION_REGION: usize = CELL_COUNT;                                       // index of the extra region containing all constellation endpoint stars
pub const SIMULATION_REGION_COUNT: usize = CELL_COUNT + 1;                                // ordinary regions plus the constellation region; calculated automatically
pub const STELLAR_DRIFT_MARGIN: f64 = PI / 720.0;                                         // include an extra 0.25° around the view to allow small star movements
pub const REFRACTION_MARGIN: f64 = 0.647 * PI / 180.0;                                    // include extra sky for atmospheric light bending (0.647°)
pub const ABERRATION_MARGIN: f64 = 22.0 * PI / (180.0 * 3600.0);                          // allow apparent shifts from observer motion (at least 22 arcseconds)
pub const QUANTIZATION_MARGIN: f64 = 0.1 * std::f64::consts::PI / (180.0 * 3600.0);       // allow rounding when matching stars to regions (0.1 arcsecond)
pub const MAX_DIRECTION_ERROR: f64 = 0.5 * std::f64::consts::PI / (180.0 * 3600.0);       // maximum position error from compact star storage (0.5 arcsecond)
pub const SINGULAR_RATIO: f64 = 1e-3;                                                     // trigger close-pass handling below 0.1% of a star’s starting distance

// Reused-sample error limits (AU = Earth–Sun distance; arcsecond = 1/3600°)
pub const PLANET_POSITION_ERROR_AU: f64 = 3e-8;                                           // maximum planet-position error compared with a fresh calculation
pub const PLANET_VELOCITY_ERROR_AU_DAY: f64 = 5e-5;                                       // maximum planet-speed error compared with a fresh calculation
pub const MOON_POSITION_ERROR_AU: f64 = 1e-9;                                             // maximum Moon-position error compared with a fresh calculation
pub const MOON_VELOCITY_ERROR_AU_DAY: f64 = 1e-6;                                         // maximum Moon-speed error compared with a fresh calculation
pub const ORIENTATION_ERROR_ARCSECONDS: f64 = 0.2;                                        // maximum Earth-axis angle error compared with a fresh calculation

// View defaults and interactive controls
pub const DEFAULT_FOV_DEGREES: f64 = 180.0;                                               // starting view width; also sets polar-projection scaling
pub const DEFAULT_CHARACTER_FPS: i64 = 24;                                                // target frames per second for character rendering unless overridden
pub const DEFAULT_PIXEL_FPS: i64 = 12;                                                    // target frames per second for pixel rendering unless overridden
pub const DEFAULT_PIXEL_TEXT_SCALE: f64 = 0.85;                                           // draw pixel text at 85% of terminal-cell size unless overridden
pub const DEFAULT_MAGNITUDE_THRESHOLD: f32 = 5.0;                                         // faintest stars shown by default; larger magnitudes are dimmer
pub const DEFAULT_SIMULATION_SPEED: f64 = 1.0;                                            // simulated seconds per real second; 1 means normal speed
pub const MIN_FOV_DEGREES: f64 = 1.0;                                                     // smallest view width allowed when zooming in
pub const PAN_STEP_FRACTION: f64 = 1.0 / 20.0;                                            // move by one twentieth of the current view width per arrow press
pub const ZOOM_FACTOR: f64 = 1.25;                                                        // divide the view width by this factor for one zoom-in step
pub const SPEED_FACTOR: f64 = 10.0;                                                       // multiply or divide simulation speed by this factor per speed-key press
pub const MAX_INTERACTIVE_SPEED: f64 = 1e12;                                              // cap keyboard-controlled speed to prevent overflow
pub const DEFAULT_CELL_ASPECT_RATIO: f64 = 2.0;                                           // assumed character-cell height divided by width if detection fails

// Drawing, labels and text layout
pub const MAX_IMAGE_PIXELS: usize = 16_777_216;                                           // maximum width × height allowed for a pixel image
pub const PIXEL_BACKGROUND_RGBA: [u8; 4] = [3, 6, 14, 255];                               // dark sky background; the last value must stay 255 for full opacity
pub const STAR_OPACITY_REFERENCE_MAGNITUDE: f64 = 0.0;                                    // stars this bright or brighter start at full opacity
pub const STAR_OPACITY_MAGNITUDE_SCALE: f64 = 0.12;                                        // larger values make opacity fall faster as stars get dimmer
pub const STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES: f64 = 180.0;                            // keep the base star brightness at this view width or wider
pub const STAR_BRIGHTNESS_ZOOM_POWER: f64 = 0.8;                                          // halving the field of view multiplies opacity by 2 raised to this power
pub const MIN_STAR_PIXEL_OPACITY: f32 = 0.05;                                             // raise nonempty star pixels to this opacity after all stars are blended
pub const DYNAMIC_NAME_COUNT: usize = 5;                                                  // label at most this many visible stars; planets and Moon are separate
pub const MAX_CACHED_GLYPHS: usize = 512;                                                 // maximum saved character images before clearing the glyph cache
pub const METADATA_PANEL_WIDTH: usize = 45;                                               // width of the metadata text panel, in character columns
pub const METADATA_TAB_WIDTH: usize = 8;                                                  // spacing between tab stops in the metadata panel
pub const METADATA_VALUE_COLUMN: usize = 16;                                              // earliest column where metadata values begin
pub const KEYS_COLUMN_WIDTH: usize = 18;                                                  // space reserved for key names in the help text

// Timing smoothing and bounded diagnostic history
pub const TIMING_EMA_FACTOR: f64 = 0.95;                                                  // keep 95% of the old timing average and add 5% of the new value
pub const MAX_MEMORY_EVENTS_PER_STEP: usize = 128;                                        // maximum memory-operation records retained for one pipeline step
pub const MAX_TRACE_STEPS: usize = 1024;                                                  // maximum recorded steps in each retained startup or frame trace
pub const MAX_TRACE_DEPTH: usize = 32;                                                    // maximum nesting of recorded pipeline steps
pub const MAX_TRACE_EVENTS: usize = 8192;                                                 // maximum memory-operation records in each retained trace
pub const MAX_TRACE_DETAILS: usize = 4096;                                                // maximum detail messages in each retained trace
pub const MAX_TRACE_TEXT_BYTES: usize = 256 * 1024;                                       // maximum combined detail-text bytes in each retained trace
pub const MAX_DETAIL_BYTES: usize = 4096;                                                 // maximum bytes kept from one detail message
pub const MAX_TRACE_INVENTORIES: usize = 1;                                               // maximum memory snapshots in each retained trace
pub const MAX_TIMING_PATHS: usize = 512;                                                  // maximum distinct step paths with smoothed timing averages
pub const MAX_AGGREGATE_PATHS: usize = 1024;                                              // maximum distinct step paths with accumulated diagnostic totals

// Memory inventory and table preview limits
pub const INVENTORY_MAX_ROWS: usize = 4096;                                               // maximum rows in a memory-usage report
pub const INVENTORY_MAX_DEPTH: usize = 16;                                                // maximum nesting inspected while measuring memory usage
pub const INVENTORY_MAX_CHILDREN: usize = 128;                                            // maximum entries inspected inside one container
pub const INVENTORY_DETAIL_CHILDREN: usize = 4;                                           // show these first entries individually, then group the remaining totals
pub const INVENTORY_MAX_VISITS: usize = 8192;                                             // maximum total entries visited during one memory inspection
pub const TABLE_PREVIEW_EDGE_ROWS: usize = 10;                                            // show this many rows from each end of a debug table
pub const MAX_TABLE_TEXT_CHARS: usize = 160;                                              // maximum characters in a debug table’s descriptive note
pub const TABLE_BYTE_PREVIEW_CHUNK_SIZE: usize = 64;                                      // bytes represented by one row when previewing raw data or text
pub const MAX_CELL_CHARS: usize = 160;                                                    // maximum characters displayed in one debug-table cell
pub const MAX_NESTED_ITEMS: usize = 4;                                                    // maximum items previewed inside a nested value
pub const MAX_PREVIEW_DEPTH: usize = 3;                                                   // maximum nesting shown inside a debug-table value
