//! Headless rendering of complete frames for fixed dates and locations.

use std::f64::consts::PI;

use astroterm::astro::{Observer, datetime_to_julian_date, greenwich_mean_sidereal_time, parse_utc_datetime};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::projection::{ProjectionKind, View, ViewCenter};
use astroterm::scene::{
    RenderOptions, draw_azimuthal_grid, draw_cardinal_directions, draw_constellations, draw_horizon_labels,
    draw_horizon_line, draw_moon, draw_planets, draw_stars,
};
use astroterm::sky::{Sky, update_moon, update_planet_positions, update_star_positions};

const ASCII: RenderOptions = RenderOptions {
    unicode: false,
    braille: false,
    color: false,
    magnitude_threshold: 5.0,
    label_threshold: 0.25,
};

/// Build and position the sky at `datetime` (UTC) for an observer at the given latitude and longitude in degrees.
fn build_sky_at(datetime: &str, latitude: f64, longitude: f64) -> Sky {
    let julian_date = datetime_to_julian_date(&parse_utc_datetime(datetime).expect("valid datetime"));
    let observer = Observer {
        latitude: latitude * PI / 180.0,
        longitude: longitude * PI / 180.0,
    };
    let sidereal_time = greenwich_mean_sidereal_time(julian_date);

    let mut sky = Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
    update_star_positions(&mut sky.stars, julian_date, sidereal_time, &observer);
    update_planet_positions(&mut sky.planets, julian_date, sidereal_time, &observer);
    update_moon(&mut sky.moon, julian_date, sidereal_time, &observer);
    sky
}

/// Draw a frame the same way the application does.
fn draw_frame(canvas: &mut Canvas, view: &View, options: &RenderOptions, sky: &Sky, grid: bool) {
    if view.is_facing() {
        draw_horizon_line(canvas, view, options);
    }
    draw_stars(canvas, view, options, sky);
    draw_constellations(canvas, view, options, sky);
    draw_planets(canvas, view, options, &sky.planets);
    draw_moon(canvas, view, options, sky);
    if view.is_facing() {
        draw_horizon_labels(canvas, view, options);
    } else if grid {
        draw_azimuthal_grid(canvas, options);
    } else {
        draw_cardinal_directions(canvas, options);
    }
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
        .stars
        .iter()
        .find(|star| star.appearance.label == Some("Polaris"))
        .expect("Polaris is named");
    assert!(polaris.position.altitude > 89.0 * PI / 180.0);

    let mut canvas = Canvas::new(41, 81);
    let options = RenderOptions {
        magnitude_threshold: 2.1,
        ..ASCII
    }; // Polaris and brighter
    draw_stars(&mut canvas, &View::default(), &options, &sky);
    let near_center = (18..=22).flat_map(|row| (37..=43).map(move |col| (row, col)));
    let glyphs: String = near_center
        .filter_map(|(row, col)| canvas.cell(row, col))
        .map(|cell| cell.symbol)
        .collect();
    assert!(
        glyphs.contains(polaris.appearance.ascii),
        "Polaris glyph near the center: {glyphs:?}"
    );
}

#[test]
fn overhead_frame_draws_stars_constellations_and_directions() {
    let sky = build_sky_at("2025-01-02T12:00:00", 1.29, 103.85); // Singapore
    let mut canvas = Canvas::new(41, 81);
    draw_frame(&mut canvas, &View::default(), &ASCII, &sky, false);

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
        ASCII,
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
            draw_frame(&mut canvas, view, options, &sky, true);
            assert_eq!(canvas.to_lines().len(), height);
        }
    }
}
