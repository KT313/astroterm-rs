//! Deterministic visual regressions. These preserve behavior; they do not establish astronomical accuracy.

#[path = "support/canvas.rs"]
mod canvas_snapshot;

#[path = "support/frame.rs"]
mod frame;
use astroterm::astro::{Observer, datetime_to_julian_date, parse_utc_datetime};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::projection::{ProjectionKind, View, ViewCenter};
use astroterm::scene::RenderOptions;
use astroterm::sky::{Sky, refract_sky_positions, update_sky_positions};
use astroterm::timing::StepTimes;
use frame::draw_sky_scene;

#[test]
fn scenes_preserve_glyphs_colors_and_wide_cell_occupancy() {
    let date = datetime_to_julian_date(&parse_utc_datetime("2025-03-01T11:00:00").unwrap());
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    }; // Tokyo
    let mut sky = Sky::from_catalog(&load_embedded_catalog().unwrap());
    update_sky_positions(&mut sky, date, &observer, 5.0, &mut StepTimes::default());
    let defaults = RenderOptions {
        unicode: false,
        braille: false,
        color: false,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    };
    for (name, facing, fov, equidistant, unicode, braille, grid, refraction, dynamic_names) in [
        ("zenith_ascii", false, 180.0, false, false, false, false, false, true),
        ("zenith_unicode", false, 180.0, false, true, false, false, false, true),
        (
            "zenith_braille_grid",
            false,
            180.0,
            false,
            true,
            true,
            true,
            false,
            true,
        ),
        ("facing_ascii", true, 120.0, false, false, false, false, false, true),
        ("facing_refracted", true, 120.0, false, true, true, false, true, true),
        ("zoom_dynamic", true, 20.0, false, true, false, false, false, true),
        (
            "zoom_without_dynamic",
            true,
            20.0,
            false,
            true,
            false,
            false,
            false,
            false,
        ),
        ("minimum_fov", true, 1.0, false, true, true, false, false, true),
        ("stereographic_359", true, 359.0, false, true, true, false, false, true),
        ("equidistant_360", true, 360.0, true, true, true, false, false, true),
    ] {
        let view = View {
            center: if facing {
                ViewCenter::Facing {
                    azimuth: 225_f64.to_radians(),
                    tilt: 30_f64.to_radians(),
                }
            } else {
                ViewCenter::Zenith
            },
            projection: if equidistant {
                ProjectionKind::Equidistant
            } else {
                ProjectionKind::Stereographic
            },
            fov_degrees: fov,
        };
        let mut frame_sky = sky.clone();
        if refraction {
            refract_sky_positions(&mut frame_sky);
        }
        let options = RenderOptions {
            unicode,
            braille,
            color: unicode,
            grid,
            dynamic_names,
            ..defaults
        };
        let mut canvas = Canvas::new(25, 51);
        draw_sky_scene(&mut canvas, &view, &options, &frame_sky);
        insta::assert_snapshot!(name, canvas_snapshot::describe_canvas(&canvas));
    }
}

#[test]
fn snapshot_format_distinguishes_color_and_continuation() {
    use astroterm::canvas::Color;
    let mut canvas = Canvas::new(1, 4);
    canvas.put_char(0, 1, '🌕', Some(Color::Yellow));
    let snapshot = canvas_snapshot::describe_canvas(&canvas);
    assert!(snapshot.contains("|.YY.|"));
    assert!(snapshot.contains("|..>.|"));
    insta::assert_snapshot!("wide_glyph", snapshot);
}

#[test]
fn filtered_updates_match_full_updates_across_threshold_changes() {
    let catalog = load_embedded_catalog().unwrap();
    let mut filtered = Sky::from_catalog(&catalog);
    let mut full = filtered.clone();
    let observer = Observer {
        latitude: 0.6,
        longitude: 2.4,
    };
    let view = View::default();
    let mut filtered_canvas = Canvas::new(41, 81);
    let mut full_canvas = Canvas::new(41, 81);
    for (step, threshold) in [-2.0, 5.0, 8.0, 0.0, 5.0].into_iter().enumerate() {
        let date = 2451545.0 + step as f64 * 1000.0;
        update_sky_positions(&mut filtered, date, &observer, threshold, &mut StepTimes::default());
        update_sky_positions(&mut full, date, &observer, f64::INFINITY, &mut StepTimes::default());
        refract_sky_positions(&mut filtered);
        refract_sky_positions(&mut full);
        let options = RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            constellations: true,
            grid: false,
            magnitude_threshold: threshold,
            label_threshold: 0.25,
            dynamic_names: true,
        };
        draw_sky_scene(&mut filtered_canvas, &view, &options, &filtered);
        draw_sky_scene(&mut full_canvas, &view, &options, &full);
        assert_eq!(
            canvas_snapshot::describe_canvas(&filtered_canvas),
            canvas_snapshot::describe_canvas(&full_canvas)
        );
    }
}

#[test]
fn accuracy_warning_is_stable_at_all_class_and_computational_boundaries() {
    use astroterm::astro::{J2000, accuracy::*};
    let mut sky = Sky::from_catalog(&load_embedded_catalog().unwrap());
    let options = RenderOptions {
        unicode: true,
        braille: false,
        color: true,
        constellations: false,
        grid: false,
        magnitude_threshold: -100.0,
        label_threshold: -100.0,
        dynamic_names: false,
    };
    let mut description = String::new();
    let mut cases = vec![("today", J2000)];
    for (name, range) in [
        ("stars", STAR_VALIDATED_INTERVAL.unwrap()),
        ("planets", PLANET_VALIDATED_INTERVAL.unwrap()),
        ("moon", MOON_VALIDATED_INTERVAL.unwrap()),
        ("computation", COMPUTATIONAL_INTERVAL),
    ] {
        for (edge, date) in [
            ("before start", range.start_tt.next_down()),
            ("at start", range.start_tt),
            ("before end", range.end_tt.next_down()),
            ("at end", range.end_tt),
        ] {
            description.push_str(&format!("{name} {edge}: {}\n", needs_accuracy_warning(date)));
            cases.push((name, date));
        }
    }
    for (name, tt) in cases {
        sky.outside_accuracy_range = needs_accuracy_warning(tt);
        for (width, expected) in [(64, ACCURACY_WARNING), (12, "Some positio")] {
            let mut canvas = Canvas::new(3, width);
            draw_sky_scene(&mut canvas, &View::default(), &options, &sky);
            if sky.outside_accuracy_range {
                assert!(canvas.to_lines()[2].starts_with(expected), "{name} {tt}");
                assert_eq!(canvas.cell(2, 0).unwrap().color, Some(astroterm::canvas::Color::Yellow));
            } else {
                assert!(!canvas.to_lines()[2].starts_with("Some"));
            }
        }
    }
    // Preserve both the message and its color without repeating every identical boundary canvas.
    sky.outside_accuracy_range = true;
    let mut canvas = Canvas::new(3, 64);
    draw_sky_scene(&mut canvas, &View::default(), &options, &sky);
    description.push_str(&canvas_snapshot::describe_canvas(&canvas));
    insta::assert_snapshot!("accuracy_warning", description);
}
