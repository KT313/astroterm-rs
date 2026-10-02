//! Drawing the sky onto a [`Canvas`] of character cells: celestial objects, orientation overlays and the metadata
//! panel. This is the character-grid renderer; it only reads the sky model.

mod appearance;
mod bodies;
mod overlays;
mod panel;

pub use appearance::{Appearance, select_moon_appearance, select_planet_appearance, select_star_appearance};
pub use bodies::{draw_constellations, draw_moon, draw_planets, draw_stars};
pub use overlays::{draw_azimuthal_grid, draw_cardinal_directions, draw_horizon_labels, draw_horizon_line};
pub use panel::draw_metadata_panel;

use crate::canvas::{Canvas, Color, draw_line_ascii, draw_line_smooth};
use crate::projection::{Polar, View};
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

/// The (row, column) canvas cell of a point on the unit disk of the view plane. Row 0 is the top.
fn polar_to_canvas_cell(canvas: &Canvas, polar: Polar) -> (i32, i32) {
    let radius_y = (canvas.height() as f64 - 1.0) / 2.0;
    let radius_x = (canvas.width() as f64 - 1.0) / 2.0;

    // sin(π) and cos(π/2) aren't exactly 0: snap them so both sides of an axis round the same way
    let snap = |value: f64| if value.abs() < 1e-12 { 0.0 } else { value };
    let (sin_theta, cos_theta) = (snap(polar.theta.sin()), snap(polar.theta.cos()));

    let row = polar.radius * -radius_y * sin_theta + radius_y; // y-axis is flipped in screen space
    let col = polar.radius * radius_x * cos_theta + radius_x;
    (row.round() as i32, col.round() as i32)
}

/// Draw a line in the style of the options (smooth Unicode or ASCII).
fn draw_line(canvas: &mut Canvas, options: &RenderOptions, start: (i32, i32), end: (i32, i32)) {
    if options.unicode {
        draw_line_smooth(canvas, start.0, start.1, end.0, end.1);
    } else {
        draw_line_ascii(canvas, start.0, start.1, end.0, end.1);
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;

    #[test]
    fn polar_to_canvas_cell_maps_disk_to_grid() {
        let canvas = Canvas::new(100, 100);
        let cell = |radius, theta| polar_to_canvas_cell(&canvas, Polar { radius, theta });
        assert_eq!(cell(0.0, 0.0), (50, 50));
        assert_eq!(cell(1.0, FRAC_PI_2), (0, 50));
        assert_eq!(cell(1.0, -FRAC_PI_2), (99, 50));

        // even height: left and right edges land on the same row
        let canvas = Canvas::new(40, 90);
        let (row_left, _) = polar_to_canvas_cell(&canvas, Polar { radius: 1.0, theta: PI });
        let (row_right, _) = polar_to_canvas_cell(
            &canvas,
            Polar {
                radius: 1.0,
                theta: 0.0,
            },
        );
        assert_eq!(row_left, row_right);
    }
}
