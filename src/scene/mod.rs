//! Drawing the sky onto a [`Canvas`] of character cells: celestial objects, orientation overlays and the metadata
//! panel. This is the character-grid renderer; it only reads the sky model.

mod appearance;
mod bodies;
pub mod cached;
mod diagnostics;
pub(crate) mod memory;
pub(crate) use bodies::select_dynamically_named_stars;
mod overlays;
mod panel;
pub mod pixels;
pub(crate) mod prepared;
pub mod raster_text;

pub use appearance::{format_star_label, select_moon_appearance, select_planet_appearance, select_star_appearance};
pub use bodies::{draw_constellations, draw_moon, draw_planets, draw_stars};
pub use overlays::{draw_azimuthal_grid, draw_cardinal_directions, draw_horizon_labels, draw_horizon_line};
pub use panel::draw_metadata_panel;

use crate::canvas::{Canvas, Color, draw_line_ascii, draw_line_smooth};
use crate::{timing::memory::{Access, BufferId, BufferShape, IndexDomain, Operation}, scene::memory::{describe_canvas}};

use crate::model::projection::ProjectedSky;

/// Rendering choices that apply to the whole scene.
use crate::model::rendering::RenderOptions;

/// Draw the sky as seen in `view` onto the canvas, back to front.
pub fn draw_sky_scene(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    draw_sky_scene_with_times(canvas, options, sky, &mut crate::timing::StepTimes::default());
}

pub(crate) fn draw_sky_scene_with_times(
    canvas: &mut Canvas,
    options: &RenderOptions,
    sky: &ProjectedSky<'_>,
    times: &mut crate::timing::StepTimes,
) {
    draw_sky_scene_prepared(canvas, options, sky, times, None);
}

pub(crate) fn draw_sky_scene_prepared(
    canvas: &mut Canvas,
    options: &RenderOptions,
    sky: &ProjectedSky<'_>,
    times: &mut crate::timing::StepTimes,
    prepared: Option<&crate::model::rendering::PreparedScene>,
) {
    times.measure("Canvas initialization", || canvas.clear());
    {
        times.record_borrow(BufferId::CharacterFrame, Access::Writable, || describe_canvas(canvas));
        times.record_shape(BufferId::CharacterFrame, Operation::Clear, None, || describe_canvas(canvas)); // fills existing cells with blanks; length is unchanged
    }

    // the horizon first in the facing view, so objects are drawn on top of it
    if sky.facing {
        times.measure("Raster horizon", || draw_horizon_line(canvas, options, sky.horizon));
    }

    // celestial objects
    times.measure("Raster stars", || {
        bodies::draw_stars_prepared(canvas, options, sky, prepared)
    });
    {
        times.record_borrow(BufferId::ProjectedView, Access::ReadOnly, || BufferShape::unknown(IndexDomain::DrawOrder));
        if let Some(prepared) = prepared { times.record_borrow(BufferId::PreparedDisplay, Access::ReadOnly, || BufferShape::vector(&prepared.stars, IndexDomain::Catalog)); }
        times.record_borrow(BufferId::CharacterFrame, Access::Writable, || describe_canvas(canvas));
    }
    if options.constellations {
        times.measure("Raster constellations", || draw_constellations(canvas, options, sky));
    }
    times.measure("Raster planets", || draw_planets(canvas, options, sky.planets));
    times.measure("Raster moon", || draw_moon(canvas, options, sky));

    // orientation aids
    times.measure("Orientation labels", || {
        if sky.facing {
            draw_horizon_labels(canvas, options, sky.horizon_labels);
        } else if options.grid {
            draw_azimuthal_grid(canvas, options);
        } else {
            draw_cardinal_directions(canvas, options);
        }
    });
    diagnostics::describe_scene(sky, options, times);

    // reserve the final row for one stable coverage message, independent of panning
    times.measure("Coverage notice", || {
        if sky.outside_accuracy_range && canvas.height() > 0 {
            let row = canvas.height() as i32 - 1;
            for col in 0..canvas.width() {
                canvas.put_char(row, col as i32, ' ', None);
            }
            canvas.put_str_truncated(row, 0, crate::astro::accuracy::ACCURACY_WARNING, Some(Color::Yellow));
        }
    });
    times.describe("Coverage notice", || {
        format!(
            "warning={}; output rows={}",
            sky.outside_accuracy_range,
            usize::from(sky.outside_accuracy_range && canvas.height() > 0)
        )
    });
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
    use crate::model::projection::{Polar, ProjectionViewport as Viewport};
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
