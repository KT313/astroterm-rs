//! Orientation aids: the azimuthal grid and compass letters of the zenith view, and the horizon line and labels of
//! the facing view.

use std::cmp::Reverse;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::astro::Horizontal;
use crate::canvas::{Canvas, Color};
use crate::projection::{Polar, View, ViewCenter};

use super::{EDGE_TOLERANCE, RenderOptions, draw_line, polar_to_canvas_cell};

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

/// Trace the visible part of the horizon in the facing view.
pub fn draw_horizon_line(canvas: &mut Canvas, view: &View, options: &RenderOptions) {
    let ViewCenter::Facing {
        azimuth: facing_azimuth,
        tilt,
    } = view.center
    else {
        return;
    };
    let Some(half_range) = compute_visible_horizon_half_range(view.fov_degrees, tilt) else {
        return;
    };

    // sample the horizon at 4 points per column (empirical)
    let sample_count = 4 * canvas.width() as i32;
    let start_azimuth = facing_azimuth - half_range;
    let step = 2.0 * half_range / f64::from(sample_count);
    let project_horizon = |azimuth: f64| view.project(Horizontal { azimuth, altitude: 0.0 });

    // join samples into segments once they are 4 columns or 2 rows apart (empirical), since the line functions can't
    // draw the slope of tiny segments
    let mut previous = project_horizon(start_azimuth);
    let mut segment_start: Option<(i32, i32)> = None;
    for index in 1..=sample_count {
        let current = project_horizon(start_azimuth + f64::from(index) * step);
        let previous_visible = previous.radius <= 1.0 + EDGE_TOLERANCE;
        let visible = current.radius <= 1.0 + EDGE_TOLERANCE;

        if previous_visible || visible {
            let start = *segment_start.get_or_insert_with(|| polar_to_canvas_cell(canvas, clamp_to_edge(previous)));
            let end = polar_to_canvas_cell(canvas, clamp_to_edge(current));
            let far_enough = (end.1 - start.1).abs() >= 4 || (end.0 - start.0).abs() >= 2;
            if !visible || index == sample_count || far_enough {
                if end != start {
                    draw_line(canvas, options, start, end); // the line functions skip zero-length segments
                }
                segment_start = visible.then_some(end);
            }
        }
        previous = current;
    }
}

/// Label the compass directions on the horizon, and the zenith and nadir, where they are in view.
pub fn draw_horizon_labels(canvas: &mut Canvas, view: &View, options: &RenderOptions) {
    const DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let color = options.select_color(Some(Color::Blue));

    // the 8 main directions, except on the very edge where labels get cut off
    for (index, label) in DIRECTIONS.iter().enumerate() {
        let azimuth = index as f64 * TAU / DIRECTIONS.len() as f64;
        let polar = view.project(Horizontal { azimuth, altitude: 0.0 });
        if polar.radius >= 1.0 - EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = polar_to_canvas_cell(canvas, polar);
        canvas.put_str_truncated(row, col - (label.len() as i32 - 1) / 2, label, color);
    }

    // zenith and nadir, edge included
    for (label, altitude) in [("Zenith", FRAC_PI_2), ("Nadir", -FRAC_PI_2)] {
        if (altitude + view.tilt()).abs() < EDGE_TOLERANCE {
            continue; // directly behind the view (tilt ±90° at fov 360°), it would sit on an arbitrary edge point
        }
        let polar = view.project(Horizontal { azimuth: 0.0, altitude });
        if polar.radius > 1.0 + EDGE_TOLERANCE {
            continue;
        }
        let (row, col) = polar_to_canvas_cell(canvas, clamp_to_edge(polar));
        canvas.put_str_truncated(row, col - label.len() as i32 / 2, label, color);
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

/// Half the azimuth range of the horizon that is in view, or `None` if the horizon is out of view.
///
/// A horizon point at azimuth offset Δ is c away from the view center, with cos(c) = cos(tilt)·cos(Δ). It is in view
/// while c <= fov/2.
fn compute_visible_horizon_half_range(fov_degrees: f64, tilt: f64) -> Option<f64> {
    let cos_half_fov = (fov_degrees.to_radians() / 2.0).cos();
    let cos_tilt = tilt.cos();
    if cos_tilt < 1e-9 {
        return Some(PI); // looking straight up or down: the horizon is a circle around the center
    }
    if cos_half_fov >= cos_tilt {
        return None;
    }
    // pad 5% so the line reaches the edge; clamped since cos(fov/2) < 0 above 180°, and kept off the point directly
    // behind, which has an arbitrary direction in the equidistant projection
    Some((PI - 1e-6).min((cos_half_fov / cos_tilt).max(-1.0).acos() * 1.05))
}

/// Pull points just outside the unit circle back onto it.
fn clamp_to_edge(polar: Polar) -> Polar {
    Polar {
        radius: polar.radius.min(1.0),
        ..polar
    }
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
    use crate::projection::ProjectionKind;

    const OPTIONS: RenderOptions = RenderOptions {
        unicode: false,
        braille: false,
        color: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
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
