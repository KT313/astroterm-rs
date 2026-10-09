//! Render settings and cached display/key records.
use crate::rows::row_columns;
use crate::canvas::{Canvas, Color};
use crate::model::{ProjectedPlanet, ProjectedMoon, ProjectedConstellation, ProjectionViewport as Viewport};

/// Rendering choices that apply to the whole scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Use Unicode glyphs instead of ASCII.
    pub unicode: bool,
    /// Draw constellation lines with braille dots (requires `unicode`).
    pub braille: bool,
    /// Use terminal colors.
    pub color: bool,
    /// Draw constellation stick figures.
    pub constellations: bool,
    /// Draw an azimuthal grid in the overhead view (instead of compass letters).
    pub grid: bool,
    /// Only draw stars at least this bright (magnitude at most this value).
    pub magnitude_threshold: f64,
    /// Label up to DYNAMIC_NAME_COUNT brightest visible stars, independently of solar-system labels.
    pub dynamic_names: bool,
}

impl RenderOptions {
    /// The color to draw with, if colors are enabled.
    pub(crate) fn select_color(&self, color: Option<Color>) -> Option<Color> {
        if self.color { color } else { None }
    }
}

/// Where the canvas sits on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalViewport {
    pub origin_row: u16,
    pub origin_col: u16,
    pub height: usize,
    pub width: usize,
}

/// The canvases drawn each frame: the square sky view, and an optional panel drawn over it in the top left corner of
/// the screen (cut off at the screen edges).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sky: Canvas,
    pub panel: Option<Canvas>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct SceneKey {
    pub(crate) production: Option<ProductionRasterKey>, // trusted regional dependencies; exact callers leave this empty
    pub(crate) viewport: Viewport, // compare small settings before the potentially large drawing inputs
    pub(crate) pixel_fov_degrees: Option<f64>,
    pub(crate) facing: bool,
    pub(crate) warning: bool,
    pub(crate) brightness_warning: bool,
    pub(crate) options: RenderOptions,
    pub(crate) canvas_size: Option<(usize, usize)>,
    pub(crate) stars: StarKeys,
    pub(crate) planets: Vec<ProjectedPlanet>,
    pub(crate) moon: Option<ProjectedMoon>,
    pub(crate) constellations: Vec<ProjectedConstellation>,
    pub(crate) horizon: Vec<[(i32, i32); 2]>,
    pub(crate) labels: Vec<((i32, i32), &'static str)>,
}

/// One straight-alpha star pixel: RGB channels and opacity are independent floats in 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StarPixel {
    pub(crate) rgb: [f32; 3],
    pub(crate) opacity: f32,
}
row_columns!(StarPixel { rgb, opacity });
// rgb = unscaled red/green/blue in 0..1; opacity = combined star coverage, also in 0..1.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PixelStarKey {
    pub(crate) cell: (i32, i32),
    pub(crate) magnitude: f64,
    pub(crate) color: [u8; 3],
}
row_columns!(PixelStarKey { cell, magnitude, color });

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CharacterStarKey {
    pub(crate) cell: (i32, i32),
    pub(crate) glyph: char,
    pub(crate) color: Option<Color>,
}
row_columns!(CharacterStarKey { cell, glyph, color });

#[derive(Clone, PartialEq)]
pub(crate) enum StarKeys {
    Pixels(Vec<PixelStarKey>),
    Characters {
        glyphs: Vec<CharacterStarKey>,
        labels: Vec<(usize, String)>,
    },
}

/// How an object is drawn: a glyph for each character set, an optional label next to it, and an optional color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance<'a> {
    pub ascii: char,
    pub unicode: char,
    pub label: Option<&'a str>,
    pub color: Option<Color>,
}

pub(crate) struct Glyph {
    pub(crate) metrics: fontdue::Metrics,
    pub(crate) coverage: Vec<u8>,
}

/// A sealed projection handoff: callers can read its sky but cannot replace data under its versions.
pub struct RenderProjection<'a> {
    pub(crate) sky: crate::model::ProjectedSky<'a>,
    pub(crate) source: (u64, u64), // projection owner and catalog/observation replacement revision
    pub(crate) regions: &'a [crate::model::ObservedRegion],
    pub(crate) assembled: &'a [(usize, usize, usize, u64, u64)],
    pub(crate) geometry: [u64; 3], // bodies, constellation lines, horizon
}
impl<'a> RenderProjection<'a> {
    pub fn sky(&self) -> &crate::model::ProjectedSky<'a> { &self.sky }
}

#[derive(Clone, PartialEq)]
pub(crate) struct ProductionRasterKey {
    pub source: (u64, u64),
    pub geometry: [u64; 3],
    pub regions: Vec<RasterRegion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RasterRegion {
    pub observed: crate::model::ObservedRegion, // includes membership, magnitudes, corrections and current row spans
    pub cells: u64,
    pub order: u64,
}
row_columns!(RasterRegion { observed, cells, order });

/// Publication marker for retained rendering results; revisions belong to their owning PixelState.
/// An input-triggered rebuild may conservatively advance this; it is not Cache's exact value generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderResultVersion {
    revision: u64,
    ready: bool,
}
impl RenderResultVersion {
    pub fn current(self) -> Option<u64> { self.ready.then_some(self.revision) }
    pub fn invalidate(&mut self) { self.ready = false; }
    pub fn publish(&mut self, changed: bool) {
        if changed || self.revision == 0 { self.revision = self.revision.checked_add(1).expect("render result revision exhausted"); }
        self.ready = true;
    }
}
row_columns!(RenderResultVersion { revision, ready });

/// Complete inputs to one retained RGB frame. Versions are local to the same PixelState owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelFrameKey {
    pub sky_version: u64,
    pub text_version: u64,
    pub dimensions: (u32, u32),
    pub screen: [u16; 4],
    pub sky_area: [u16; 4],
    pub font: (u16, u16),
    pub text_cell: (u16, u16),
    pub background: [u8; 4],
}
row_columns!(PixelFrameKey { sky_version, text_version, dimensions, screen, sky_area, font, text_cell, background });

/// Exact Kitty upload parameters; image IDs cannot share already encoded command bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KittyEncodingKey {
    pub rgb_version: u64,
    pub dimensions: (u32, u32),
    pub compression: u8, // 0 unknown, 1 supported, 2 unsupported; preserves capability changes as well as byte format
    pub tmux: bool,
    pub image_id: u32,
}
row_columns!(KittyEncodingKey { rgb_version, dimensions, compression, tmux, image_id });

/// A completed render can either submit output or leave an already displayed Kitty frame untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderOutcome { Presented, ReusedDisplayedFrame }
impl RenderOutcome {
    pub fn was_presented(self) -> bool { self == Self::Presented }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KittyDisplayKey {
    pub shared_memory: bool,
    pub rgb_version: u64,
    pub dimensions: (u32, u32),
    pub screen: [u16; 4],
    pub compression: u8,
    pub tmux: bool,
}
row_columns!(KittyDisplayKey { shared_memory, rgb_version, dimensions, screen, compression, tmux });
