//! Drawing stars, constellation figures, planets and the Moon.

use std::borrow::Cow;

use crate::astro::{Horizontal, offset_towards};
use crate::canvas::{Canvas, draw_line_braille};
use crate::projection::{Polar, View};
use crate::sky::{Planet, Sky};

use super::appearance::{
    Appearance, format_star_label, select_moon_appearance, select_planet_appearance, select_star_appearance,
};
use super::{RenderOptions, draw_line, polar_to_canvas_cell};

/// With dynamic names, the brightest stars in view are named until at least this many objects in view have labels.
const DYNAMIC_NAME_COUNT: usize = 5;

/// Draw the stars bright enough for the threshold, dimmest first. Named stars brighter than the label threshold get
/// labels, and with dynamic names also the brightest stars in view when few objects in view have labels.
pub fn draw_stars(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    let dynamically_named = if options.dynamic_names {
        select_dynamically_named_stars(view, options, sky)
    } else {
        Vec::new()
    };

    for &index in &sky.stars_by_brightness {
        let star = &sky.stars[index];
        if star.magnitude > options.magnitude_threshold {
            continue;
        }
        let label = if dynamically_named.contains(&index) {
            Some(format_star_label(star, options.unicode))
        } else if star.magnitude <= options.label_threshold {
            star.name.map(Cow::Borrowed)
        } else {
            None
        };
        draw_object(
            canvas,
            view,
            options,
            &select_star_appearance(star),
            star.position,
            label.as_deref(),
        );
    }
}

/// Indices of the stars to name in addition to the usual labels: the brightest drawn stars in view without a label,
/// until at least [`DYNAMIC_NAME_COUNT`] objects in view (the Sun, planets, Moon and labelled stars) have labels.
fn select_dynamically_named_stars(view: &View, options: &RenderOptions, sky: &Sky) -> Vec<usize> {
    // the Sun, planets and Moon are always labelled
    let in_view = |position| is_on_disk(view.project(position));
    let planets_in_view = sky.planets.iter().filter(|planet| in_view(planet.position)).count();
    let mut labelled = planets_in_view + usize::from(in_view(sky.moon.position));

    // then stars, brightest first: ones labelled anyway only count, the others get a name
    let mut selected = Vec::new();
    for &index in sky.stars_by_brightness.iter().rev() {
        let star = &sky.stars[index];
        if labelled >= DYNAMIC_NAME_COUNT || star.magnitude > options.magnitude_threshold {
            break; // enough labels, or this and all following stars are too dim to be drawn
        }
        if !in_view(star.position) {
            continue;
        }
        if star.magnitude > options.label_threshold || star.name.is_none() {
            selected.push(index);
        }
        labelled += 1;
    }
    selected
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
        let appearance = select_planet_appearance(planet.kind);
        draw_object(canvas, view, options, &appearance, planet.position, appearance.label);
    }
}

/// Draw the Moon. Its Unicode glyph shows its phase, lit from the side the Sun is on as seen in this view.
pub fn draw_moon(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
    let moon = &sky.moon;
    let lit_on_right = is_lit_on_right(view, moon.position, sky.sun().position);
    let appearance = select_moon_appearance(moon.phase, lit_on_right);
    draw_object(canvas, view, options, &appearance, moon.position, appearance.label);
}

/// Whether, in this view, the direction from the Moon towards the Sun points to the right of the screen.
fn is_lit_on_right(view: &View, moon: Horizontal, sun: Horizontal) -> bool {
    let towards_sun = offset_towards(moon, sun, 1_f64.to_radians());
    let (moon_x, _) = view.project(moon).to_cartesian();
    let (towards_sun_x, _) = view.project(towards_sun).to_cartesian();
    towards_sun_x > moon_x
}

/// Whether a projected point is in view (on the unit disk).
fn is_on_disk(polar: Polar) -> bool {
    polar.radius.abs() <= 1.0
}

/// Draw an object's glyph, and its label (if any) up and to the right of it. Objects out of view are skipped.
fn draw_object(
    canvas: &mut Canvas,
    view: &View,
    options: &RenderOptions,
    appearance: &Appearance,
    position: Horizontal,
    label: Option<&str>,
) {
    let polar = view.project(position);
    if !is_on_disk(polar) {
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
    if let Some(label) = label {
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
    use crate::catalog::load_embedded_catalog;
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
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: false,
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

    const DYNAMIC: RenderOptions = RenderOptions {
        dynamic_names: true,
        ..ASCII
    };

    /// The real sky with every object at the nadir (out of the overhead view), except the given stars, which are placed
    /// on a ring around the zenith.
    fn place_in_view(visible: &[usize]) -> Sky {
        let mut sky = Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
        let nadir = horizontal(0.0, -90.0);
        sky.stars.iter_mut().for_each(|star| star.position = nadir);
        sky.planets.iter_mut().for_each(|planet| planet.position = nadir);
        sky.moon.position = nadir;
        for (step, &index) in visible.iter().enumerate() {
            sky.stars[index].position = horizontal(step as f64 * 40.0, 60.0);
        }
        sky
    }

    /// Seven stars, brightest first, all dimmer than the label threshold; the brightest has no proper name.
    fn pick_unlabelled_stars(sky: &Sky) -> Vec<usize> {
        let brightest_first: Vec<usize> = sky.stars_by_brightness.iter().rev().copied().collect();
        let unnamed = brightest_first
            .iter()
            .position(|&index| sky.stars[index].name.is_none() && sky.stars[index].magnitude > ASCII.label_threshold)
            .unwrap();
        brightest_first[unnamed..unnamed + 7].to_vec()
    }

    #[test]
    fn dynamic_names_go_to_the_five_brightest_stars_in_view() {
        let stars = pick_unlabelled_stars(&place_in_view(&[]));
        let sky = place_in_view(&stars);
        assert_eq!(
            select_dynamically_named_stars(&View::default(), &DYNAMIC, &sky),
            stars[..5]
        );

        // the unnamed one shows its catalog number
        let mut canvas = Canvas::new(41, 81);
        draw_stars(&mut canvas, &View::default(), &DYNAMIC, &sky);
        let label = format!("HR {}", stars[0] + 1); // the embedded catalog is indexed by HR number
        assert!(canvas.to_lines().iter().any(|line| line.contains(&label)), "{label}");
    }

    #[test]
    fn planets_moon_and_labelled_stars_in_view_count_toward_the_five() {
        let stars = pick_unlabelled_stars(&place_in_view(&[]));
        let vega = 7000; // brighter than the label threshold, and named
        let mut sky = place_in_view(&[&stars[..], &[vega]].concat());
        sky.planets[3].position = horizontal(100.0, 70.0);
        sky.moon.position = horizontal(200.0, 70.0);
        assert_eq!(
            select_dynamically_named_stars(&View::default(), &DYNAMIC, &sky),
            stars[..2]
        );
    }

    #[test]
    fn stars_too_dim_to_be_drawn_are_never_named() {
        let stars = pick_unlabelled_stars(&place_in_view(&[]));
        let sky = place_in_view(&stars);
        let options = RenderOptions {
            magnitude_threshold: sky.stars[stars[2]].magnitude,
            ..DYNAMIC
        };
        let drawn: Vec<usize> = stars
            .iter()
            .copied()
            .filter(|&index| sky.stars[index].magnitude <= options.magnitude_threshold)
            .collect();
        assert!(drawn.len() >= 3 && drawn.len() < 5, "{drawn:?}");
        assert_eq!(select_dynamically_named_stars(&View::default(), &options, &sky), drawn);
    }

    #[test]
    fn without_dynamic_names_only_bright_named_stars_are_labelled() {
        let stars = pick_unlabelled_stars(&place_in_view(&[]));
        let sky = place_in_view(&stars);
        let mut canvas = Canvas::new(41, 81);
        draw_stars(&mut canvas, &View::default(), &ASCII, &sky);
        let text = canvas.to_lines().concat();
        assert!(
            !text
                .chars()
                .any(|symbol| symbol.is_ascii_alphabetic() && symbol != 'O' && symbol != 'o')
        );
    }
}
