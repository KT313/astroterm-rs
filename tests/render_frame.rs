//! Headless rendering of complete frames for fixed dates and locations.

use std::f64::consts::PI;

#[path = "support/frame.rs"]
mod frame;
use astroterm::astro::{Observer, datetime_to_julian_date, parse_utc_datetime};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{ProjectionKind, View, ViewCenter, RenderOptions};
use astroterm::scene::select_star_appearance;
use astroterm::model::Sky;
use astroterm::sky::update_sky_positions;
use astroterm::timing::StepTimes;
use frame::draw_sky_scene;

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

/// Build and position the sky at `datetime` (UTC) for an observer at the given latitude and longitude in degrees.
fn build_sky_at(datetime: &str, latitude: f64, longitude: f64) -> Sky {
    let julian_date = datetime_to_julian_date(&parse_utc_datetime(datetime).expect("valid datetime"));
    let observer = Observer {
        latitude: latitude * PI / 180.0,
        longitude: longitude * PI / 180.0,
    };
    let mut sky = astroterm::sky::create_sky_from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
    update_sky_positions(
        &mut sky,
        julian_date,
        &observer,
        f64::INFINITY,
        &mut StepTimes::default(),
    );
    sky
}

fn count_non_blank(canvas: &Canvas) -> usize {
    canvas
        .to_lines()
        .iter()
        .flat_map(|line| line.chars())
        .filter(|symbol| *symbol != ' ')
        .count()
}

#[test]
fn polaris_is_near_the_center_at_the_north_pole() {
    let sky = build_sky_at("2025-01-02T12:00:00", 90.0, 0.0);
    let polaris = sky
        .star_views()
        .find(|star| sky.star_name(star) == Some("Polaris"))
        .expect("Polaris is named");
    assert!(polaris.horizontal_position().altitude > 89.0 * PI / 180.0);

    let mut canvas = Canvas::new(41, 81);
    let options = RenderOptions {
        magnitude_threshold: 2.1,
        ..ASCII
    }; // Polaris and brighter
    let projected_data = astroterm::projection::project_sky(
        &sky,
        &View::default(),
        astroterm::model::ProjectionViewport {
            height: canvas.height(),
            width: canvas.width(),
        },
    );
    let projected = projected_data.view(&sky);
    astroterm::scene::draw_stars(&mut canvas, &options, &projected);
    let near_center = (18..=22).flat_map(|row| (37..=43).map(move |col| (row, col)));
    let glyphs: String = near_center
        .filter_map(|(row, col)| canvas.cell(row, col))
        .map(|cell| cell.symbol)
        .collect();
    assert!(
        glyphs.contains(select_star_appearance(&polaris, &sky.names).ascii),
        "Polaris glyph near the center: {glyphs:?}"
    );
}

#[test]
fn overhead_frame_draws_stars_constellations_and_directions() {
    let sky = build_sky_at("2025-01-02T12:00:00", 1.29, 103.85); // Singapore
    let mut canvas = Canvas::new(41, 81);
    draw_sky_scene(&mut canvas, &View::default(), &ASCII, &sky);

    let lines = canvas.to_lines();
    assert_eq!(lines[0].chars().nth(40), Some('N'));
    assert_eq!(lines[40].chars().nth(40), Some('S'));
    assert!(lines.iter().any(|line| line.contains('+')), "constellation markers");
    assert!(count_non_blank(&canvas) > 200);
}

#[test]
fn every_view_and_style_renders_within_bounds() {
    let sky = build_sky_at("1969-07-16T08:00:00", 28.573469, -80.651070); // Apollo 11 launch
    let views = [
        View::default(),
        View {
            fov_degrees: 90.0,
            ..View::default()
        },
        View {
            center: ViewCenter::Facing {
                azimuth: 337.5_f64.to_radians(),
                tilt: 20_f64.to_radians(),
            },
            projection: ProjectionKind::Stereographic,
            fov_degrees: 120.0,
        },
        View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: -90_f64.to_radians(),
            },
            projection: ProjectionKind::Stereographic,
            fov_degrees: 359.0,
        },
        View {
            center: ViewCenter::Facing {
                azimuth: 337.5_f64.to_radians(),
                tilt: 0.0,
            },
            projection: ProjectionKind::Equidistant,
            fov_degrees: 360.0,
        },
        View {
            projection: ProjectionKind::Equidistant,
            fov_degrees: 360.0,
            ..View::default()
        },
    ];
    let styles = [
        RenderOptions { grid: true, ..ASCII },
        RenderOptions { unicode: true, ..ASCII },
        RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            ..ASCII
        },
    ];
    for (view, options) in views
        .iter()
        .flat_map(|view| styles.iter().map(move |options| (view, options)))
    {
        for (height, width) in [(41, 81), (7, 13), (1, 1), (0, 0)] {
            let mut canvas = Canvas::new(height, width);
            draw_sky_scene(&mut canvas, view, options, &sky);
            assert_eq!(canvas.to_lines().len(), height);
        }
    }
}

/// Labels on the canvas: words with a lowercase letter (names) and catalog numbers ("HR 1234"); compass letters are
/// all uppercase and not counted.
fn count_labels(canvas: &Canvas) -> usize {
    let text = canvas.to_lines().join(" ");
    let is_label = |word: &&str| word.chars().any(char::is_lowercase) || *word == "HR";
    text.split_whitespace().filter(is_label).count()
}

#[test]
fn dynamic_names_label_stars_in_a_zoomed_view() {
    let sky = build_sky_at("2025-03-01T11:00:00", 35.69, 139.69); // Tokyo, 20:00 local time
    let view = View {
        center: ViewCenter::Facing {
            azimuth: 225_f64.to_radians(),
            tilt: 30_f64.to_radians(),
        },
        projection: ProjectionKind::Stereographic,
        fov_degrees: 20.0,
    };
    let count_labels_with = |dynamic_names| {
        let mut canvas = Canvas::new(41, 81);
        draw_sky_scene(&mut canvas, &view, &RenderOptions { dynamic_names, ..ASCII }, &sky);
        count_labels(&canvas)
    };
    let (without, with) = (count_labels_with(false), count_labels_with(true));
    assert!(
        with > without && with >= 3,
        "labels without dynamic names: {without}, with: {with}"
    );
}
