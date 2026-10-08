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
    pub(crate) stars: StarKeys,
    pub(crate) planets: Vec<ProjectedPlanet>,
    pub(crate) moon: Option<ProjectedMoon>,
    pub(crate) constellations: Vec<ProjectedConstellation>,
    pub(crate) horizon: Vec<[(i32, i32); 2]>,
    pub(crate) labels: Vec<((i32, i32), &'static str)>,
    pub(crate) viewport: Viewport,
    pub(crate) facing: bool,
    pub(crate) warning: bool,
    pub(crate) brightness_warning: bool,
    pub(crate) options: RenderOptions,
    pub(crate) canvas_size: Option<(usize, usize)>,
}

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
