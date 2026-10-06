//! Orientation aids: the azimuthal grid and compass letters of the zenith view, and the horizon line and labels of
//! the facing view.

use std::cmp::Reverse;

use crate::canvas::{Canvas, Color};

use crate::model::RenderOptions;
use super::draw_line;

/// Draw spokes from the center to the edge every few degrees of azimuth, labelled with their angle. The spacing
/// adapts to the canvas size.
pub fn draw_azimuthal_grid(canvas: &mut Canvas, options: &RenderOptions) {
    // half sizes of the canvas, and the spoke spacing
    let radius_rows = ((canvas.height() as f64 - 1.0) / 2.0).round() as i32;
    let radius_cols = ((canvas.width() as f64 - 1.0) / 2.0).round() as i32;
    let step = select_grid_step(radius_rows);

    // within a quadrant, spokes on coarser divisions of 90° are drawn last so they end up on top
    let mut angles: Vec<i32> = (0..=90 / step).map(|index| index * step).collect();
    angles.sort_by_key(|&angle| Reverse(90 / greatest_common_divisor(angle, 90)));

    // spokes and labels in all four quadrants
    for quadrant in 0..4 {
        for &base_angle in &angles {
            let angle = base_angle + 90 * quadrant;
            let radians = f64::from(angle).to_radians();
            let row = radius_rows - (f64::from(radius_rows) * radians.sin()).round() as i32;
            let col = radius_cols + (f64::from(radius_cols) * radians.cos()).round() as i32;
            draw_line(canvas, options, (row, col), (radius_rows, radius_cols));

            let label = angle.to_string();
            let label_offset = if col < radius_cols {
                0
            } else {
                -(label.len() as i32 - 1)
            }; // keep labels inside
            canvas.put_str_truncated(row, col + label_offset, &label, None);
        }
    }
}

/// Draw N, E, S and W at the edges of the zenith view. East is on the left, as seen looking up.
pub fn draw_cardinal_directions(canvas: &mut Canvas, options: &RenderOptions) {
    let color = options.select_color(Some(Color::Blue));
    let (height, width) = (canvas.height() as i32, canvas.width() as i32);
    let half_rows = (f64::from(height - 1) / 2.0).round() as i32;
    let half_cols = (f64::from(width - 1) / 2.0).round() as i32;

    canvas.put_char(0, half_cols, 'N', color);
    canvas.put_char(half_rows, width - 1, 'W', color);
    canvas.put_char(height - 1, half_cols, 'S', color);
    canvas.put_char(half_rows, 0, 'E', color);
}

/// Draw the prepared horizon behind celestial objects.
pub fn draw_horizon_line(canvas: &mut Canvas, options: &RenderOptions, lines: &[[(i32, i32); 2]]) {
    for &[start, end] in lines {
        draw_line(canvas, options, start, end);
    }
}

/// Draw prepared compass and vertical labels.
pub fn draw_horizon_labels(canvas: &mut Canvas, options: &RenderOptions, labels: &[((i32, i32), &'static str)]) {
    let color = options.select_color(Some(Color::Blue));
    for &((row, col), label) in labels {
        canvas.put_str_truncated(row, col, label, color);
    }
}

/// Spoke spacing in degrees: the finest step (a multiple of 5 and a factor of 90) whose spokes are at least 10 rows
/// apart at the edge.
fn select_grid_step(radius_rows: i32) -> i32 {
    const STEPS: [i32; 5] = [10, 15, 30, 45, 90];
    const MIN_ROWS_APART: i32 = 10;
    let rows_apart = |step: i32| (f64::from(radius_rows) * f64::from(step).to_radians().sin()).round() as i32;
    STEPS
        .into_iter()
        .find(|&step| rows_apart(step) >= MIN_ROWS_APART)
        .unwrap_or(90)
}

fn greatest_common_divisor(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectionViewport as Viewport, View, ViewCenter};
    use crate::projection::{project_horizon_labels, project_horizon_line};
    use std::f64::consts::{FRAC_PI_2, PI};
    fn draw_horizon_line(canvas: &mut Canvas, view: &View, options: &RenderOptions) {
        let lines = project_horizon_line(
            view,
            Viewport {
                height: canvas.height(),
                width: canvas.width(),
            },
        );
        super::draw_horizon_line(canvas, options, &lines);
    }
    fn draw_horizon_labels(canvas: &mut Canvas, view: &View, options: &RenderOptions) {
        let labels = project_horizon_labels(
            view,
            Viewport {
                height: canvas.height(),
                width: canvas.width(),
            },
        );
        super::draw_horizon_labels(canvas, options, &labels);
    }
    fn compute_visible_horizon_half_range(fov: f64, tilt: f64) -> Option<f64> {
        crate::projection::compute_visible_horizon_half_range(fov, tilt)
    }

    use crate::model::ProjectionKind;

    const OPTIONS: RenderOptions = RenderOptions {
        unicode: false,
        braille: false,
        color: false,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: false,
    };

    #[test]
    fn grid_angles_on_coarse_divisions_are_drawn_last() {
        let mut angles: Vec<i32> = (0..=9).map(|index| index * 10).collect();
        angles.sort_by_key(|&angle| Reverse(90 / greatest_common_divisor(angle, 90)));
        assert_eq!(angles, [10, 20, 40, 50, 70, 80, 30, 60, 0, 90]);
    }

    #[test]
    fn grid_step_adapts_to_canvas_size() {
        assert_eq!(select_grid_step(100), 10);
        assert_eq!(select_grid_step(30), 30);
        assert_eq!(select_grid_step(5), 90);
    }

    #[test]
    fn cardinal_directions_sit_on_the_edges() {
        let mut canvas = Canvas::new(5, 9);
        draw_cardinal_directions(&mut canvas, &OPTIONS);
        assert_eq!(
            canvas.to_lines(),
            ["    N    ", "         ", "E       W", "         ", "    S    "]
        );
    }

    #[test]
    fn horizon_is_hidden_when_looking_far_above_it() {
        assert!(compute_visible_horizon_half_range(60.0, 80f64.to_radians()).is_none());
        assert_eq!(compute_visible_horizon_half_range(60.0, FRAC_PI_2), Some(PI));
        assert!(compute_visible_horizon_half_range(180.0, 0.0).is_some());
    }

    #[test]
    fn horizon_runs_across_the_middle_of_a_level_view() {
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            projection: ProjectionKind::Stereographic,
            fov_degrees: 180.0,
        };
        let mut canvas = Canvas::new(11, 21);
        draw_horizon_line(&mut canvas, &view, &OPTIONS);
        assert_eq!(canvas.to_lines()[5], "-".repeat(21));
        assert!(
            canvas
                .to_lines()
                .iter()
                .enumerate()
                .all(|(row, line)| row == 5 || line.trim().is_empty())
        );
    }

    #[test]
    fn horizon_labels_show_directions_in_view() {
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            projection: ProjectionKind::Stereographic,
            fov_degrees: 180.0,
        };
        let mut canvas = Canvas::new(11, 21);
        draw_horizon_labels(&mut canvas, &view, &OPTIONS);
        let lines = canvas.to_lines();
        assert!(lines[5].contains('N') && lines[5].contains("NE") && lines[5].contains("NW"));
        assert!(!lines[5].contains('S')); // behind the observer
        assert!(lines[0].contains("Zenith") && lines[10].contains("Nadir"));
    }
}
