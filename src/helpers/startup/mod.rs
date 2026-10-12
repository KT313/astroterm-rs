//! Validate inputs, load the selected catalog into the sky tables and open the terminal.
use std::io;
use std::process::ExitCode;
use astroterm::catalog::datasets::{Dataset, DatasetDirectories};
use astroterm::catalog::{City, load_embedded_cities};
use astroterm::cli::{Arguments, build_config, write_bash_completions};
use astroterm::model::{Config, GraphicsProtocol, SkyTables};
use astroterm::present::Display;
use astroterm::timing::StepTimes;
use ratatui_image::picker::ProtocolType;
use super::{finish_requested_report, report_failure};

/// Load the embedded city table, recording diagnostics and reporting any failure.
pub(crate) fn load_cities(step_times: &mut StepTimes) -> Result<Vec<City>, ExitCode> {
    let cities = match step_times.measure("City loading", load_embedded_cities) {
        Ok(cities) => cities,
        Err(error) => return Err(report_failure(error)),
    };
    step_times.describe("City loading", || format!("cities loaded={}", cities.len()));
    Ok(cities)
}

/// Print Bash completions without entering the rendering pipeline.
pub(crate) fn print_bash_completions(cities: &[City]) -> ExitCode {
    match write_bash_completions(&mut io::stdout().lock(), cities) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Build the runtime configuration and report invalid arguments before returning failure.
pub(crate) fn validate_arguments(arguments: Arguments, cities: &[City]) -> Result<Config, ExitCode> {
    build_config(arguments, cities).map_err(report_failure)
}

/// Load the selected catalog, split it into the sky tables of the frame loop and record the startup
/// diagnostics. The prepared catalog is dropped once the tables are built: the tables are the only copy.
pub(crate) fn load_catalog(config: &Config, directories: &DatasetDirectories, times: &mut StepTimes) -> Result<SkyTables, ExitCode> {
    let loaded = {
        let mut progress = astroterm::terminal::open_startup_progress(config.no_progress);
        let result = times.measure_steps("Dataset loading", |times| {
            astroterm::sky::load_sky_catalog_with_times(config.dataset.as_ref(), config.dataset_parallel_blocks, directories, &mut progress, times)
        });
        let result = result.and_then(|catalog| {
            catalog.catalog.validate_exception_support()?;                                                // reject unsupported sparse data before the frame loop
            times.measure_steps("Group tables", |times| astroterm::sky::build_sky_tables(&catalog, &mut progress, times)).map(|tables| (catalog, tables))
        });
        progress.finish(result.is_ok()); // stop all startup drawing before terminal setup or error reporting
        result
    };
    let allow_empty = matches!(config.dataset, Some(Dataset::GaiaGdr3 { files: 0 } | Dataset::GaiaGdr3Stars { stars: 0 })); // either zero selection explicitly requests no stars
    let loaded = match loaded {
        Ok((catalog, _)) if catalog.catalog.stars.is_empty() && !allow_empty => Err(io::Error::other("dataset contains no stars")),
        Ok(loaded) => Ok(loaded),
        Err(error) => Err(io::Error::other(error)),
    };
    let (catalog, tables) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => return Err(finish_requested_report(Err(error), config, times, &mut io::stdout().lock(), &mut io::stderr().lock())),
    };
    if matches!(config.dataset, Some(Dataset::GaiaGdr3 { .. } | Dataset::GaiaGdr3Stars { .. })) && config.render.constellations {
        eprintln!("Gaia DR3: constellation figures are unavailable; only the selected Gaia sources are loaded.");
    }
    let catalog = &catalog.catalog;
    if config.dataset.is_some() { eprintln!("Catalog: {} stars loaded.", catalog.stars.len()); } // the embedded catalog loads silently, as before
    times.describe("Dataset loading", || format!(
        "output stars={}; constellation figures={}; unique endpoints={}; tangential fallbacks={}; group tables={}; constellation stars={}",
        catalog.stars.len(), catalog.constellations().len(), catalog.endpoint_indices().len(), catalog.singular_count,
        tables.stars.iter().filter(|(_, _, table)| !table.is_empty()).count(), tables.constellations.len(),
    ));
    eprintln!("Catalog: {} stars use tangential motion after near-collision checks.", catalog.singular_count);
    Ok(tables)                                                                                              // the prepared catalog is dropped here
}

/// Open the terminal with the configured or detected graphics protocol and describe the setup.
pub(crate) fn open_display(config: &Config, times: &mut StepTimes) -> io::Result<Display> {
    let forced = match config.graphics_protocol {
        GraphicsProtocol::Auto => None,
        GraphicsProtocol::Kitty => Some(ProtocolType::Kitty),
        GraphicsProtocol::Sixel => Some(ProtocolType::Sixel),
        GraphicsProtocol::Iterm2 => Some(ProtocolType::Iterm2),
        GraphicsProtocol::Halfblocks => Some(ProtocolType::Halfblocks),
    };
    let display = times.measure("Terminal setup", || Display::open(forced, config.terminal, config.text_scale))?;
    let info = display.info();
    times.describe("Terminal setup", || format!("protocol={:?}; frame={}x{}; cell={}x{}; metadata={}; frame-time panel={}",
        info.protocol, info.image.0, info.image.1, info.font.0, info.font.1, config.terminal.metadata_panel, config.terminal.frame_times));
    Ok(display)
}
