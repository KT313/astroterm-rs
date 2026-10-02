//! Drawing stars, constellation figures, planets and the Moon.

use crate::astro::{Horizontal, offset_towards};
use crate::canvas::{Canvas, draw_line_braille};
use crate::projection::{Polar, View};
use crate::sky::{Appearance, Planet, Sky};

use super::{RenderOptions, draw_line, polar_to_canvas_cell};

/// Radius that stands in for points projected to infinity (directly behind a stereographic view), so clipping math
/// stays finite.
const MAX_CLIP_RADIUS: f64 = 1e6;

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
            let start = view.project(sky.stars[a].position);
            let end = view.project(sky.stars[b].position);
            draw_constellation_segment(canvas, options, start, end);
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

/// Draw the visible part of a segment between two projected stars, with markers on the stars that are in view.
fn draw_constellation_segment(canvas: &mut Canvas, options: &RenderOptions, start: Polar, end: Polar) {
    let Some(clipped) = clip_segment_to_unit_disk(start, end) else {
        return;
    };
    let start_cell = polar_to_canvas_cell(canvas, clipped.start);
    let end_cell = polar_to_canvas_cell(canvas, clipped.end);

    // the line
    if options.unicode && options.braille {
        draw_line_braille(canvas, start_cell.0, start_cell.1, end_cell.0, end_cell.1);
    } else {
        draw_line(canvas, options, start_cell, end_cell);
    }

    // markers on the stars themselves, not on the edge of the view
    let marker = if options.unicode { '○' } else { '+' };
    for (cell, clipped) in [(start_cell, clipped.start_clipped), (end_cell, clipped.end_clipped)] {
        if !clipped {
            canvas.put_char(cell.0, cell.1, marker, None);
        }
    }
}

/// The part of a segment inside the unit disk, and which of its ends were cut off.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClippedSegment {
    start: Polar,
    end: Polar,
    start_clipped: bool,
    end_clipped: bool,
}

/// Clip the straight segment between two points of the view plane to the unit disk. `None` if it misses the disk.
fn clip_segment_to_unit_disk(start: Polar, end: Polar) -> Option<ClippedSegment> {
    // segments fully in view are kept as they are
    if start.radius.abs() <= 1.0 && end.radius.abs() <= 1.0 {
        return Some(ClippedSegment {
            start,
            end,
            start_clipped: false,
            end_clipped: false,
        });
    }

    // intersect P(t) = A + t·(B - A), t in [0, 1], with |P| <= 1
    let capped = |polar: Polar| {
        Polar {
            radius: polar.radius.min(MAX_CLIP_RADIUS),
            ..polar
        }
        .to_cartesian()
    };
    let ((ax, ay), (bx, by)) = (capped(start), capped(end));
    let (dx, dy) = (bx - ax, by - ay);
    let a = dx * dx + dy * dy;
    let b = 2.0 * (ax * dx + ay * dy);
    let c = ax * ax + ay * ay - 1.0;
    let discriminant = b * b - 4.0 * a * c;
    if a == 0.0 || discriminant.is_nan() || discriminant < 0.0 {
        return None; // degenerate, missing the disk, or not finite
    }

    // the visible parameter range
    let root = discriminant.sqrt();
    let t_start = ((-b - root) / (2.0 * a)).max(0.0);
    let t_end = ((-b + root) / (2.0 * a)).min(1.0);
    if t_start > t_end {
        return None;
    }
    let point_at = |t: f64| Polar::from_cartesian(ax + t * dx, ay + t * dy);
    Some(ClippedSegment {
        start: point_at(t_start),
        end: point_at(t_end),
        start_clipped: t_start > 0.0,
        end_clipped: t_end < 1.0,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::projection::ViewCenter;

    fn polar_at(x: f64, y: f64) -> Polar {
        Polar::from_cartesian(x, y)
    }

    fn assert_near(polar: Polar, x: f64, y: f64) {
        let (px, py) = polar.to_cartesian();
        assert!(
            (px - x).abs() < 1e-9 && (py - y).abs() < 1e-9,
            "({px}, {py}) vs ({x}, {y})"
        );
    }

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

    #[test]
    fn segment_inside_is_unchanged() {
        let (start, end) = (polar_at(0.1, 0.2), polar_at(-0.5, 0.3));
        let clipped = clip_segment_to_unit_disk(start, end).unwrap();
        assert_eq!((clipped.start, clipped.end), (start, end));
        assert!(!clipped.start_clipped && !clipped.end_clipped);
    }

    #[test]
    fn segment_leaving_the_disk_is_cut_where_it_crosses_the_edge() {
        // the C version clipped radially, moving the outside end to (1, 1)/√2 instead of the crossing point
        let clipped = clip_segment_to_unit_disk(polar_at(0.0, 0.5), polar_at(3.0, 0.5)).unwrap();
        assert_near(clipped.start, 0.0, 0.5);
        assert_near(clipped.end, 0.75_f64.sqrt(), 0.5);
        assert!(!clipped.start_clipped && clipped.end_clipped);

        // either end may be the one outside
        let reversed = clip_segment_to_unit_disk(polar_at(3.0, 0.5), polar_at(0.0, 0.5)).unwrap();
        assert!(reversed.start_clipped && !reversed.end_clipped);
    }

    #[test]
    fn segment_crossing_the_disk_with_both_ends_outside_keeps_the_chord() {
        let clipped = clip_segment_to_unit_disk(polar_at(-2.0, 0.0), polar_at(2.0, 0.0)).unwrap();
        assert_near(clipped.start, -1.0, 0.0);
        assert_near(clipped.end, 1.0, 0.0);
        assert!(clipped.start_clipped && clipped.end_clipped);
    }

    #[test]
    fn segment_missing_the_disk_is_dropped() {
        assert!(clip_segment_to_unit_disk(polar_at(-2.0, 1.5), polar_at(2.0, 1.5)).is_none());
        assert!(clip_segment_to_unit_disk(polar_at(1.5, 0.0), polar_at(3.0, 0.0)).is_none());
    }

    #[test]
    fn point_at_infinity_still_clips() {
        let infinite = Polar {
            radius: f64::INFINITY,
            theta: FRAC_PI_2,
        };
        let clipped = clip_segment_to_unit_disk(Polar { radius: 0.0, theta: PI }, infinite).unwrap();
        assert_near(clipped.end, 0.0, 1.0);
    }
}
