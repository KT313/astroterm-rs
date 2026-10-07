//! Measure the actual application startup loader, including download/cache behavior. Set XDG_DATA_HOME and
//! XDG_CACHE_HOME to isolated folders on Linux when testing a cold named-dataset download.
use astroterm::astro::{J2000, Observer};
use astroterm::catalog::datasets::{Dataset, DatasetDirectories};
use astroterm::model::{Sky, ProjectionViewport as Viewport, View};
use astroterm::projection::project_sky;
use astroterm::sky::{update_sky_positions, load_sky_catalog};
use astroterm::timing::StepTimes;
use std::{io, sync::Arc, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argument = std::env::args_os().nth(1).ok_or("expected a dataset name or path")?;
    let dataset = Dataset::parse(&argument).map_err(io::Error::other)?;
    let start = Instant::now();
    let catalog = load_sky_catalog(
        Some(&dataset),
        &DatasetDirectories::for_user(),
        &mut io::stderr().lock(),
    )?;
    let elapsed = start.elapsed().as_secs_f64();
    let count = catalog.catalog.stars.len();
    let mut sky = Sky::new(Arc::new(catalog.catalog));
    update_sky_positions(&mut sky, J2000, &Observer::default(), 5.0, &mut StepTimes::default());
    let projected_data = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
    let projected = projected_data.view(&sky);
    println!(
        "load_seconds={elapsed:.6} storage=owned stars={count} visible={}",
        projected.stars.len()
    );
    Ok(())
}
