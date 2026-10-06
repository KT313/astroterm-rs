//! Drawing primitives, object appearance, and catalog-bound display constants.
pub(super) mod appearance;
mod bodies;
mod overlays;
pub(super) mod pixels;
pub(super) mod prepared;

pub use appearance::{format_star_label, select_moon_appearance, select_planet_appearance, select_star_appearance};
pub use bodies::{draw_constellations, draw_moon, draw_planets, draw_stars};
pub use overlays::{draw_azimuthal_grid, draw_cardinal_directions, draw_horizon_labels, draw_horizon_line};
pub(crate) use bodies::select_dynamically_named_stars;
pub(super) use bodies::draw_stars_prepared;

use crate::canvas::{Canvas, Color, draw_line_ascii, draw_line_smooth};
use crate::model::{ProjectedSky, RenderOptions};

pub(super) fn draw_orientation_labels(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    if sky.facing {
        draw_horizon_labels(canvas, options, sky.horizon_labels);
    } else if options.grid {
        draw_azimuthal_grid(canvas, options);
    } else {
        draw_cardinal_directions(canvas, options);
    }
}

pub(super) fn draw_coverage_notice(canvas: &mut Canvas, sky: &ProjectedSky<'_>) {
    if sky.outside_accuracy_range && canvas.height() > 0 {
        let row = canvas.height() as i32 - 1;
        for col in 0..canvas.width() { canvas.put_char(row, col as i32, ' ', None); }
        canvas.put_str_truncated(row, 0, crate::astro::accuracy::ACCURACY_WARNING, Some(Color::Yellow));
    }
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
    use crate::model::{Polar, ProjectionViewport as Viewport};
    fn polar_to_canvas_cell(canvas: &Canvas, polar: Polar) -> (i32, i32) {
        crate::projection::polar_to_cell(Viewport {
            height: canvas.height(),
            width: canvas.width(),
        }, polar)
    }

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
