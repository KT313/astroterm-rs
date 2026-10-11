//! Save a representative pure raster frame for visual inspection (terminal text is a separate layer).
use astroterm::astro::{Observer, datetime_to_julian_date, parse_utc_datetime};
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{ProjectionViewport as Viewport, View, RenderOptions};
use astroterm::projection::project_sky;
use astroterm::scene::draw_pixel_sky;
use astroterm::sky::update_sky_positions;
use astroterm::timing::StepTimes;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("output PNG path");
    let mut sky = astroterm::sky::create_sky_from_catalog(&load_embedded_catalog()?).unwrap();
    let mut times = StepTimes::default();
    let observer = Observer {
        latitude: 35.69_f64.to_radians(),
        longitude: 139.69_f64.to_radians(),
    };
    let date = datetime_to_julian_date(&parse_utc_datetime("2025-03-01T11:00:00").unwrap());
    update_sky_positions(&mut sky, date, &observer, 5.0, &mut times);
    let projected_data = project_sky(
        &sky,
        &View::default(),
        Viewport {
            width: 800,
            height: 800,
        },
    );
    let projected = projected_data.view(&sky);
    let options = RenderOptions {
        unicode: true,
        braille: false,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        dynamic_names: true,
    };
    draw_pixel_sky(&projected, &options, &mut times).unwrap().save(path)?;
    Ok(())
}
