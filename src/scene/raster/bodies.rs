//! Drawing stars, constellation figures, planets and the Moon.


use crate::canvas::{Canvas, draw_line_braille};
use crate::model::{ProjectedArc, ProjectedPlanet, ProjectedSky};

use crate::model::Appearance;
use super::appearance::{format_star_label, select_moon_appearance, select_planet_appearance};
use crate::model::RenderOptions;
use super::draw_line;


/// Draw stars in prepared order; optionally label the global brightest visible stars.
pub fn draw_stars(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    let dynamically_named = select_dynamically_named_stars(options, sky);

    for (index, entry) in sky.stars.iter().enumerate() {
        if entry.star.magnitude > options.magnitude_threshold {
            continue;
        }
        let star = entry.star;
        let label = if dynamically_named.contains(&index) {
            Some(format_star_label(&star, sky.names, options.unicode))
        } else {
            None
        };
        draw_object(
            canvas,
            options,
            &super::appearance::select_star_appearance(&star, sky.names),
            entry.cell,
            label.as_deref(),
        );
    }
}

/// Select global label winners from the brightest eligible stars in each drawing region.
pub(crate) fn select_dynamically_named_stars(options: &RenderOptions, sky: &ProjectedSky<'_>) -> super::labels::StarLabels {
    super::labels::select_star_labels(options, sky, |_| true)
}

/// Draw the stick figures of all constellations whose stars are all bright enough for the threshold.
pub fn draw_constellations(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    for constellation in sky.constellations {
        if constellation.maximum_magnitude > options.magnitude_threshold {
            continue;
        }
        for arc in &constellation.arcs {
            draw_constellation_arc(canvas, options, arc);
        }
    }
}

/// Draw the Sun and the planets, outermost first so the Sun ends up on top.
pub fn draw_planets(canvas: &mut Canvas, options: &RenderOptions, planets: &[ProjectedPlanet]) {
    for planet in planets.iter().rev() {
        let appearance = select_planet_appearance(planet.kind);
        draw_object(canvas, options, &appearance, planet.cell, appearance.label);
    }
}

/// Draw the Moon. Its Unicode glyph shows its phase, lit from the side the Sun is on as seen in this view.
pub fn draw_moon(canvas: &mut Canvas, options: &RenderOptions, sky: &ProjectedSky<'_>) {
    let moon = &sky.moon;
    let appearance = select_moon_appearance(moon.phase, moon.light_direction.is_some_and(|p| p.x > 0.0));
    draw_object(canvas, options, &appearance, moon.cell, appearance.label);
}

/// Draw an object's glyph, and its label (if any) up and to the right of it. Objects out of view are skipped.
fn draw_object(
    canvas: &mut Canvas,
    options: &RenderOptions,
    appearance: &Appearance,
    cell: Option<(i32, i32)>,
    label: Option<&str>,
) {
    let Some((row, col)) = cell else {
        return;
    };
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

fn draw_constellation_arc(canvas: &mut Canvas, options: &RenderOptions, arc: &ProjectedArc) {
    let (start, end) = (arc.start, arc.end);
    for pair in arc.points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if options.unicode && options.braille {
            draw_line_braille(canvas, a.0, a.1, b.0, b.1);
        } else {
            draw_line(canvas, options, a, b);
        }
    }
    for (cell, is_star) in [(start, arc.includes_start), (end, arc.includes_end)] {
        if is_star {
            canvas.put_char(cell.0, cell.1, if options.unicode { '○' } else { '+' }, None);
        }
    }
}
#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::astro::{Horizontal, offset_towards};
    use crate::model::{ProjectionViewport as Viewport, View};
    use crate::projection::{project_constellation_segment, project_sky};
    use crate::model::Sky;
    fn viewport(canvas: &Canvas) -> Viewport {
        Viewport {
            height: canvas.height(),
            width: canvas.width(),
        }
    }
    fn draw_stars(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky) {
        super::draw_stars(canvas, options, &project_sky(sky, view, viewport(canvas)).view(sky));
    }
    fn select_dynamically_named_stars(view: &View, options: &RenderOptions, sky: &Sky) -> Vec<usize> {
        let projected_data = project_sky(sky, view, Viewport { height: 41, width: 81 });
        let projected = projected_data.view(sky);
        super::select_dynamically_named_stars(options, &projected)
            .rev()
            .map(|index| {
                sky.star_views()
                    .position(|star| star.id() == projected.stars.get(index).star.id())
                    .unwrap()
            })
            .collect()
    }
    fn draw_constellation_segment(
        canvas: &mut Canvas,
        view: &View,
        options: &RenderOptions,
        from: Horizontal,
        to: Horizontal,
    ) {
        for arc in project_constellation_segment(view, viewport(canvas), from.to_unit_vector(), to.to_unit_vector()) {
            draw_constellation_arc(canvas, options, &arc);
        }
    }
    fn is_lit_on_right(view: &View, moon: Horizontal, sun: Horizontal) -> bool {
        let offset = offset_towards(moon, sun, 1_f64.to_radians());
        crate::projection::project_horizontal(view, offset).to_cartesian().0 > crate::projection::project_horizontal(view, moon).to_cartesian().0
    }

    use crate::catalog::load_embedded_catalog;
    use crate::model::ViewCenter;

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
        let mut sky = crate::sky::create_sky_from_catalog(&load_embedded_catalog().expect("embedded catalog loads")).unwrap();
        let nadir = horizontal(0.0, -90.0).to_unit_vector();
        sky.stars.iter_mut().for_each(|star| star.position = nadir);
        sky.planets.iter_mut().for_each(|planet| planet.position = nadir);
        sky.moon.position = nadir;
        for (step, &index) in visible.iter().enumerate() {
            sky.stars[index].position = horizontal(step as f64 * 40.0, 60.0).to_unit_vector();
        }
        sky
    }

    /// Seven stars, brightest first; the first uses a catalog identifier.
    fn pick_unlabelled_stars(sky: &Sky) -> Vec<usize> {
        let mut brightest_first: Vec<usize> = (0..sky.stars.len()).collect();
        brightest_first.sort_unstable_by(|&a, &b| {
            sky.stars[a]
                .magnitude
                .total_cmp(&sky.stars[b].magnitude)
                .then_with(|| sky.star_view(b).id().cmp(&sky.star_view(a).id()))
        });
        let unnamed = brightest_first
            .iter()
            .position(|&index| {
                sky.star_name(&sky.stars[index]).is_some_and(|name| name.starts_with("HR "))
            })
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
        let label = format!("HR {}", sky.star_view(stars[0]).id().0);
        assert!(canvas.to_lines().iter().any(|line| line.contains(&label)), "{label}");
    }

    #[test]
    fn planets_and_moon_do_not_reduce_the_five_star_labels() {
        let stars = pick_unlabelled_stars(&place_in_view(&[]));
        let vega = place_in_view(&[])
            .star_views()
            .position(|star| star.id().0 == 7001)
            .unwrap(); // a bright star with a proper name
        let mut sky = place_in_view(&[&stars[..], &[vega]].concat());
        sky.planets[3].position = horizontal(100.0, 70.0).to_unit_vector();
        sky.moon.position = horizontal(200.0, 70.0).to_unit_vector();
        assert_eq!(
            select_dynamically_named_stars(&View::default(), &DYNAMIC, &sky),
            [&[vega][..], &stars[..4]].concat()
        );
    }

    #[test]
    fn newly_quantized_ties_draw_by_ascending_id_and_name_by_descending_id() {
        let mut catalog = load_embedded_catalog().unwrap();
        catalog.stars.truncate(6);
        for (index, star) in catalog.stars.iter_mut().enumerate() {
            star.id = crate::catalog::StarId(index as u32);
            star.magnitude = 5.00001 + index as f64 * 0.00001;
            star.has_data = true;
        }
        let mut sky = crate::sky::create_sky_from_catalog(&catalog).unwrap();
        for star in &mut sky.stars { star.position = horizontal(0.0, 80.0).to_unit_vector(); }
        let mut data = project_sky(&sky, &View::default(), Viewport { width: 81, height: 41 });
        for planet in &mut data.planets { planet.cell = None; }
        data.moon.cell = None;
        let projected = data.view(&sky);
        assert_eq!(projected.stars.iter().map(|s| s.star.id().0).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
        let labels = super::select_dynamically_named_stars(&DYNAMIC, &projected);
        assert_eq!(labels.rev().map(|i| projected.stars.get(i).star.id().0).collect::<Vec<_>>(), [5, 4, 3, 2, 1]);
    }

    #[test]
    fn dynamic_names_follow_current_magnitudes_instead_of_catalog_order() {
        let indices = pick_unlabelled_stars(&place_in_view(&[]));
        let mut sky = place_in_view(&indices);
        let mut catalog = load_embedded_catalog().unwrap();
        for star in &mut catalog.stars {
            star.name = None;
        }
        sky.catalog = std::sync::Arc::new(crate::sky::prepare_catalog(&catalog).unwrap().catalog);
        for (step, &index) in indices.iter().enumerate() {
            sky.stars[index].magnitude = 4.0 - step as f64;
            assert!(sky.star_name(&sky.stars[index]).unwrap().starts_with("HR "));
        }
        let expected = indices.iter().rev().take(5).copied().collect::<Vec<_>>();
        assert_eq!(
            select_dynamically_named_stars(&View::default(), &DYNAMIC, &sky),
            expected
        );
    }

    #[test]
    fn bright_proper_names_obey_the_label_switch() {
        let initial = place_in_view(&[]);
        let vega = initial.star_views().position(|star| star.id().0 == 7001).unwrap();
        let mut sky = place_in_view(&[vega]);
        let mut canvas = Canvas::new(41, 81);
        sky.stars[vega].magnitude = 0.5;
        draw_stars(&mut canvas, &View::default(), &ASCII, &sky);
        assert!(!canvas.to_lines().concat().contains("Vega"));
        sky.stars[vega].magnitude = 0.1;
        draw_stars(&mut canvas, &View::default(), &ASCII, &sky);
        assert!(!canvas.to_lines().concat().contains("Vega"));
        draw_stars(&mut canvas, &View::default(), &DYNAMIC, &sky);
        assert!(canvas.to_lines().concat().contains("Vega"));
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
    fn without_dynamic_names_no_stars_are_labelled() {
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
