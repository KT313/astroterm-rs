//! Validate inputs, load the selected catalog and open the scoped terminal session.
use std::io;
use std::process::ExitCode;
use astroterm::catalog::{City, datasets::DatasetDirectories, load_embedded_cities};
use astroterm::cli::{Arguments, build_config, write_bash_completions};
use astroterm::model::Config;
use astroterm::state::ApplicationState;
use astroterm::terminal::Renderer;
use astroterm::timing::StepTimes;
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

/// Load the selected catalog into the state and record its startup diagnostics.
pub(crate) fn load_catalog(state: &mut ApplicationState, directories: &DatasetDirectories) -> Result<(), ExitCode> {
    let catalog = {
        let mut progress = astroterm::terminal::open_startup_progress(state.config.no_progress);
        let result = state.timings.measure_steps("Dataset loading", |times| {
            astroterm::sky::load_sky_catalog_with_times(state.config.dataset.as_ref(), directories, &mut progress, times)
        });
        progress.finish(result.is_ok()); // stop all startup drawing before terminal setup or error reporting
        result
    };
    let allow_empty = state.config.dataset == Some(astroterm::catalog::datasets::Dataset::GaiaGdr3 { files: 0 }); // zero files explicitly requests no stars
    let catalog = match catalog {
        Ok(catalog) if catalog.catalog.stars.is_empty() && !allow_empty => Err(io::Error::other("dataset contains no stars")),
        Ok(catalog) => Ok(catalog),
        Err(error) => Err(io::Error::other(error)),
    };
    let catalog = match catalog {
        Ok(catalog) => catalog,
        Err(error) => return Err(finish_requested_report(Err(error), &state.config, &state.timings,
            &mut io::stdout().lock(), &mut io::stderr().lock())),
    };
    if matches!(state.config.dataset, Some(astroterm::catalog::datasets::Dataset::GaiaGdr3 { .. })) && state.config.render.constellations {
        eprintln!("Gaia DR3: constellation figures are unavailable; only the selected Gaia sources are loaded.");
    }
    state.replace_catalog(catalog);                                          // the single catalog install point
    let catalog = &state.persistent.catalog;
    if state.config.dataset.is_some() { eprintln!("Catalog: {} stars loaded.", state.star_counts.loaded); } // the embedded catalog loads silently, as before

    #[cfg(feature = "memory-diagnostics")]
    state.timings.record_memory(state.timings.last_memory_step(), || {
        use astroterm::timing::{BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
        let shape = BufferShape { len: Some(catalog.stars.len()), ..BufferShape::unknown(IndexDomain::Catalog) };
        MemoryEvent::operation(BufferId::CatalogStars, Operation::Build, None, Some(shape), shape.len, None)
    });

    state.timings.describe("Dataset loading", || format!(
        "output stars={}; constellation figures={}; unique endpoints={}; tangential fallbacks={}; storage=owned",
        catalog.stars.len(), catalog.constellations().len(), catalog.endpoint_indices().len(),
        catalog.singular_count,
    ));

    eprintln!(
        "Catalog: {} stars use tangential motion after near-collision checks.",
        catalog.singular_count
    );

    Ok(())
}

/// Open the terminal, install its working buffers and describe setup before the caller starts the frame loop.
pub(crate) fn prepare_terminal(state: &mut ApplicationState) -> io::Result<Renderer> {
    let (renderer, rendering) = state.timings.measure("Terminal setup", || {
        let config = &state.config;
        Renderer::open(config.renderer, config.graphics_protocol, config.render, config.terminal, config.text_scale)
    })?;
    state.cache.rendering = rendering;
    let config = &state.config;
    state.timings.describe("Terminal setup", || format!("renderer={:?}; projection viewport={}x{}; metadata={}; frame-time panel={}; runtime cache enabled={}", config.renderer, renderer.viewport(&state.cache.rendering).width, renderer.viewport(&state.cache.rendering).height, config.terminal.metadata_panel, config.terminal.frame_times, config.cache.enabled));
    Ok(renderer)
}
