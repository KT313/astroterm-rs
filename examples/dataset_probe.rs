//! Measure the actual application startup loader, including download/cache behavior, and the sky tables built
//! from it. Set XDG_DATA_HOME and XDG_CACHE_HOME to isolated folders on Linux when testing a cold download.
use astroterm::catalog::datasets::{Dataset, DatasetDirectories};
use astroterm::constants::MAST_PARALLEL_BLOCKS;
use astroterm::progress::NoProgress;
use astroterm::sky::{build_sky_tables, load_sky_catalog};
use astroterm::timing::StepTimes;
use std::{io, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argument = std::env::args_os().nth(1).ok_or("expected a dataset name or path")?;
    let dataset = Dataset::parse(&argument).map_err(io::Error::other)?;
    let start = Instant::now();
    let mut progress = NoProgress(io::stderr().lock());
    let catalog = load_sky_catalog(Some(&dataset), MAST_PARALLEL_BLOCKS, &DatasetDirectories::for_user(), &mut astroterm::progress::Report::live(&mut progress, &mut StepTimes::default()))?;
    let elapsed = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let tables = build_sky_tables(&catalog, &mut astroterm::progress::Report::live(&mut progress, &mut StepTimes::default()))?;
    let tables_elapsed = start.elapsed().as_secs_f64();
    let filled = tables.stars.iter().filter(|(_, _, table)| !table.is_empty()).count();
    println!(
        "load_seconds={elapsed:.6} tables_seconds={tables_elapsed:.6} stars={} constellation_stars={} group_tables={filled}",
        tables.stars.len(), tables.constellations.len()
    );
    Ok(())
}
