//! Deterministic visual regressions. These preserve behavior; they do not establish astronomical accuracy.

#[path = "support/canvas.rs"]
mod canvas_snapshot;

use astroterm::astro::{Observer, datetime_to_julian_date, parse_utc_datetime};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::projection::{ProjectionKind, View, ViewCenter};
use astroterm::scene::{RenderOptions, draw_sky_scene};
use astroterm::sky::{Sky, refract_sky_positions, update_sky_positions};
use astroterm::timing::StepTimes;

#[test]
fn scenes_preserve_glyphs_colors_and_wide_cell_occupancy() {
    let date = datetime_to_julian_date(&parse_utc_datetime("2025-03-01T11:00:00").unwrap());
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    }; // Tokyo
    let mut sky = Sky::from_catalog(&load_embedded_catalog().unwrap());
    update_sky_positions(&mut sky, date, &observer, &mut StepTimes::default());
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
