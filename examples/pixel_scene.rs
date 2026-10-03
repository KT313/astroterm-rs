//! Save a representative pure raster frame for visual inspection (terminal text is a separate layer).
use astroterm::{
    astro::{Observer, datetime_to_julian_date, parse_utc_datetime},
    catalog::load_embedded_catalog,
    projection::{View, Viewport, project_sky},
    scene::{RenderOptions, pixels::draw_pixel_sky},
    sky::{Sky, update_sky_positions},
    timing::StepTimes,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("output PNG path");
    let mut sky = Sky::from_catalog(&load_embedded_catalog()?);
    let mut times = StepTimes::default();
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };
    let date = datetime_to_julian_date(&parse_utc_datetime("2025-03-01T11:00:00").unwrap());
    update_sky_positions(&mut sky, date, &observer, 5.0, &mut times);
    let projected = project_sky(
        &sky,
        &View::default(),
        Viewport {
            width: 800,
            height: 800,
        },
    );
    let options = RenderOptions {
        unicode: true,
        braille: false,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    };
    draw_pixel_sky(&projected, &options, &mut times).unwrap().save(path)?;
    Ok(())
}
