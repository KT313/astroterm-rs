//! Drawing the sky onto a [`Canvas`] of character cells: celestial objects, orientation overlays and the metadata
//! panel. This is the character-grid renderer; it only reads the sky model.

mod appearance;
mod bodies;
mod local_time;
mod metadata;
mod overlays;

pub use appearance::{Appearance, select_moon_appearance, select_planet_appearance, select_star_appearance};
pub use bodies::{draw_constellations, draw_moon, draw_planets, draw_stars};
pub use metadata::draw_metadata;
pub use overlays::{draw_azimuthal_grid, draw_cardinal_directions, draw_horizon_labels, draw_horizon_line};

use crate::canvas::{Canvas, Color, draw_line_ascii, draw_line_smooth};
use crate::projection::{Polar, View, polar_to_cell};
use crate::sky::Sky;

/// Slack for points on the edge of the unit circle (rounding error).
const EDGE_TOLERANCE: f64 = 1e-6;

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
    pub magnitude_threshold: f32,
    /// Only label stars at least this bright.
    pub label_threshold: f32,
}

impl RenderOptions {
    /// The color to draw with, if colors are enabled.
    fn select_color(&self, color: Option<Color>) -> Option<Color> {
        if self.color { color } else { None }
    }
}

/// Draw the sky as seen in `view` onto the canvas, back to front.
pub fn draw_sky_scene(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    canvas.clear();

    // the horizon first in the facing view, so objects are drawn on top of it
    if view.is_facing() {
        draw_horizon_line(canvas, view, options);
    }

    // celestial objects
    draw_stars(canvas, view, options, sky);
    if options.constellations {
        draw_constellations(canvas, view, options, sky);
    }
    draw_planets(canvas, view, options, &sky.planets);
    draw_moon(canvas, view, options, sky);

    // orientation aids
    if view.is_facing() {
        draw_horizon_labels(canvas, view, options);
    } else if options.grid {
        draw_azimuthal_grid(canvas, options);
    } else {
        draw_cardinal_directions(canvas, options);
    }
}

/// The canvas cell of a point on the view plane.
fn polar_to_canvas_cell(canvas: &Canvas, polar: Polar) -> (i32, i32) {
    polar_to_cell(polar, canvas.height(), canvas.width())
}

/// Draw a line in the style of the options (smooth Unicode or ASCII).
fn draw_line(canvas: &mut Canvas, options: &RenderOptions, start: (i32, i32), end: (i32, i32)) {
    if options.unicode {
        draw_line_smooth(canvas, start.0, start.1, end.0, end.1);
    } else {
        draw_line_ascii(canvas, start.0, start.1, end.0, end.1);
    }
}
