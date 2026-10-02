//! Drawing stars, constellation figures, planets and the Moon.

use crate::astro::{Horizontal, offset_towards};
use crate::canvas::{Canvas, draw_line_braille};
use crate::projection::{Polar, View};
use crate::sky::{Appearance, Planet, Sky};

use super::{RenderOptions, draw_line, polar_to_canvas_cell};

/// Draw the stars bright enough for the threshold, dimmest first. Only the brightest stars get labels.
pub fn draw_stars(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    for &index in &sky.stars_by_brightness {
        let star = &sky.stars[index];
        if star.magnitude > options.magnitude_threshold {
            continue;
        }
        let show_label = star.magnitude <= options.label_threshold;
        draw_object(canvas, view, options, &star.appearance, star.position, show_label);
    }
}

/// Draw the stick figures of all constellations whose stars are all bright enough for the threshold.
pub fn draw_constellations(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    for constellation in &sky.constellations {
        let mut star_indices = constellation.segments.iter().flatten();
        if star_indices.any(|&index| sky.stars[index].magnitude > options.magnitude_threshold) {
            continue;
        }
        for &[a, b] in &constellation.segments {
            draw_constellation_segment(canvas, view, options, sky.stars[a].position, sky.stars[b].position);
        }
    }
}

/// Draw the Sun and the planets, outermost first so the Sun ends up on top.
pub fn draw_planets(canvas: &mut Canvas, view: &View, options: &RenderOptions, planets: &[Planet]) {
    for planet in planets.iter().rev() {
        draw_object(canvas, view, options, &planet.appearance, planet.position, true);
    }
}

/// Draw the Moon. Its Unicode glyph shows its phase, lit from the side the Sun is on as seen in this view.
pub fn draw_moon(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    let moon = &sky.moon;
    let lit_on_right = is_lit_on_right(view, moon.position, sky.sun().position);
    let appearance = Appearance {
        unicode: moon.phase.glyph(lit_on_right),
        ..moon.appearance
    };
    draw_object(canvas, view, options, &appearance, moon.position, true);
}

/// Whether, in this view, the direction from the Moon towards the Sun points to the right of the screen.
fn is_lit_on_right(view: &View, moon: Horizontal, sun: Horizontal) -> bool {
    let towards_sun = offset_towards(moon, sun, 1_f64.to_radians());
    let (moon_x, _) = view.project(moon).to_cartesian();
    let (towards_sun_x, _) = view.project(towards_sun).to_cartesian();
    towards_sun_x > moon_x
}

/// Draw an object's glyph, and its label up and to the right of it. Objects out of view are skipped.
fn draw_object(
    canvas: &mut Canvas,
    view: &View,
    options: &RenderOptions,
    appearance: &Appearance,
    position: Horizontal,
    show_label: bool,
) {
    let polar = view.project(position);
    if polar.radius.abs() > 1.0 {
        return;
    }
    let (row, col) = polar_to_canvas_cell(canvas, polar);
    let color = options.select_color(appearance.color);

    let glyph = if options.unicode {
        appearance.unicode
    } else {
        appearance.ascii
    };
    canvas.put_char(row, col, glyph, color);
    if let (true, Some(label)) = (show_label, appearance.label) {
        canvas.put_str_truncated(row - 1, col + 1, label, color);
    }
}

/// Draw the visible parts of the great-circle arc between two stars, each as a straight line between where it enters
/// and leaves the view, with markers on the stars that are in view.
fn draw_constellation_segment(
    canvas: &mut Canvas,
    view: &View,
    options: &RenderOptions,
    from: Horizontal,
    to: Horizontal,
) {
    let marker = if options.unicode { '○' } else { '+' };
    for part in view.find_visible_arc_parts(from, to) {
        // ends of the visible part, pulled onto the edge where rounding puts them just outside
        let project_on_arc = |angle: f64| {
            let polar = view.project(offset_towards(from, to, angle));
            polar_to_canvas_cell(
                canvas,
                Polar {
                    radius: polar.radius.min(1.0),
                    ..polar
                },
            )
        };
        let (start_cell, end_cell) = (project_on_arc(part.start), project_on_arc(part.end));

        // the line
        if options.unicode && options.braille {
            draw_line_braille(canvas, start_cell.0, start_cell.1, end_cell.0, end_cell.1);
        } else {
            draw_line(canvas, options, start_cell, end_cell);
        }

        // markers on the stars themselves, not on the edge of the view
        for (cell, is_star) in [(start_cell, part.includes_start), (end_cell, part.includes_end)] {
            if is_star {
                canvas.put_char(cell.0, cell.1, marker, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::projection::ViewCenter;

    #[test]
    fn lit_side_follows_the_sun_on_screen() {
        let moon = Horizontal {
            azimuth: 180_f64.to_radians(),
            altitude: 30_f64.to_radians(),
        };
        let sun_in_the_west = Horizontal {
            azimuth: 260_f64.to_radians(),
            altitude: -10_f64.to_radians(),
        };
        let facing_south = View {
            center: ViewCenter::Facing { azimuth: PI, tilt: 0.0 },
            ..View::default()
        };
        let facing_north = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: FRAC_PI_2,
            },
            ..View::default()
        };

        assert!(is_lit_on_right(&facing_south, moon, sun_in_the_west)); // West is on the right when facing South
        assert!(is_lit_on_right(&View::default(), moon, sun_in_the_west)); // ... and in the overhead view (W on the right)
        assert!(!is_lit_on_right(&facing_north, moon, sun_in_the_west)); // ... but on the left looking up, facing North
    }

    const ASCII: RenderOptions = RenderOptions {
        unicode: false,
        braille: false,
        color: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
    };

    fn horizontal(azimuth_degrees: f64, altitude_degrees: f64) -> Horizontal {
        Horizontal {
            azimuth: azimuth_degrees.to_radians(),
            altitude: altitude_degrees.to_radians(),
        }
    }

    fn count_marks(canvas: &Canvas) -> (usize, usize) {
        let text: String = canvas.to_lines().concat();
        (
            text.chars().filter(|&symbol| symbol == '+').count(),
            text.chars().filter(|&symbol| symbol != ' ').count(),
        )
    }

    #[test]
    fn segment_in_view_has_markers_on_both_stars() {
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            fov_degrees: 90.0,
            ..View::default()
        };
        let mut canvas = Canvas::new(21, 41);
        draw_constellation_segment(
            &mut canvas,
            &view,
            &ASCII,
            horizontal(-20.0, 5.0),
            horizontal(20.0, -5.0),
        );
        let (markers, marks) = count_marks(&canvas);
        assert_eq!(markers, 2);
        assert!(marks > 10);
    }

    #[test]
    fn segment_leaving_the_view_has_one_marker_and_reaches_the_edge() {
        let view = View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            fov_degrees: 90.0,
            ..View::default()
        };
        let mut canvas = Canvas::new(21, 41);
        draw_constellation_segment(&mut canvas, &view, &ASCII, horizontal(0.0, 0.0), horizontal(90.0, 0.0));
        assert_eq!(count_marks(&canvas).0, 1);
        assert_ne!(canvas.cell(10, 40).map(|cell| cell.symbol), Some(' ')); // the right edge on the horizon
    }

    #[test]
    fn segment_hidden_behind_a_wide_view_draws_nothing() {
        // the reported artifact: two stars on opposite sides of the nadir project far outside the circle on opposite
        // sides, and a straight line between them crossed the whole display
        let view = View {
            fov_degrees: 270.0,
            ..View::default()
        };
        let mut canvas = Canvas::new(21, 41);
        draw_constellation_segment(
            &mut canvas,
            &view,
            &ASCII,
            horizontal(10.0, -85.0),
            horizontal(190.0, -85.0),
        );
        assert_eq!(count_marks(&canvas), (0, 0));
    }
}
