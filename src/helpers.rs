//! Application startup, terminal lifetime, diagnostics and exit reporting.

use std::fmt::Display;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use astroterm::catalog::{City, datasets::DatasetDirectories, load_embedded_cities};
use astroterm::cli::{Arguments, Config, build_config, write_bash_completions};
use astroterm::sky::Sky;
use astroterm::terminal::Renderer;
use astroterm::timing::StepTimes;

use crate::pipeline::run_render_loop;

/// Load the embedded city table, recording diagnostics and reporting any failure.
pub(super) fn load_cities(step_times: &mut StepTimes) -> Result<Vec<City>, ExitCode> {
    let cities = match step_times.measure("City loading", load_embedded_cities) {
        Ok(cities) => cities,
        Err(error) => return Err(report_failure(error)),
    };
    step_times.describe("City loading", || format!("cities loaded={}", cities.len()));
    Ok(cities)
}

/// Print Bash completions without entering the rendering pipeline.
pub(super) fn print_bash_completions(cities: &[City]) -> ExitCode {
    match write_bash_completions(&mut io::stdout().lock(), cities) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Build the runtime configuration and report invalid arguments before returning failure.
pub(super) fn validate_arguments(arguments: Arguments, cities: &[City]) -> Result<Config, ExitCode> {
    build_config(arguments, cities).map_err(report_failure)
}

/// Load the selected catalog and record its startup diagnostics.
pub(super) fn load_catalog_sky(
    config: &Config,
    directories: &DatasetDirectories,
    step_times: &mut StepTimes,
) -> Result<Sky, ExitCode> {
    let catalog = step_times.measure_steps("Dataset loading", |times| {
        astroterm::sky::cache::load_sky_catalog_with_times(
            config.dataset.as_ref(),
            directories,
            &mut io::stderr().lock(),
            times,
        )
    });
    let sky = match catalog {
        Ok(catalog) => Sky::new(Arc::new(catalog)),
        Err(error) => return Err(report_failure(error)),
    };

    step_times.describe("Dataset loading", || format!(
        "output stars={}; constellation figures={}; unique endpoints={}; always-checked stars={}; tangential fallbacks={}; mapped={}",
        sky.catalog.stars.len(), sky.catalog.constellations.len(), sky.catalog.endpoint_indices.len(),
        sky.catalog.always_checked.len(), sky.catalog.singular_count, sky.catalog.stars.is_mapped(),
    ));

    eprintln!(
        "Catalog: {} stars use tangential motion after near-collision checks.",
        sky.catalog.singular_count
    );

    Ok(sky)
}

/// Open the renderer and restore the terminal before returning the render-loop result.
pub(super) fn render_in_terminal(config: &Config, sky: &mut Sky, step_times: &mut StepTimes) -> io::Result<()> {
    step_times
        .measure("Terminal setup", || {
            Renderer::open(
                config.renderer,
                config.graphics_protocol,
                config.render,
                config.terminal,
                config.text_scale,
            )
        })
        .and_then(|mut renderer| {
            step_times.describe("Terminal setup", || format!("renderer={:?}; projection viewport={}x{}; metadata={}; frame-time panel={}; runtime cache enabled={}", config.renderer, renderer.viewport().width, renderer.viewport().height, config.terminal.metadata_panel, config.terminal.frame_times, config.cache.enabled));
            run_render_loop(config, sky, &mut renderer, step_times)
        })
}

/// Print a successful single-frame trace or report the rendering failure after terminal cleanup.
pub(super) fn finish_rendering(result: io::Result<()>, config: &Config, step_times: &StepTimes) -> ExitCode {
    if result.is_ok()
        && config.debug_singleframe
        && let Err(error) = step_times.trace().unwrap().write_report(&mut io::stdout().lock())
    {
        return report_failure(error);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(error),
    }
}

/// Print an error and return a failing exit code.
fn report_failure(error: impl Display) -> ExitCode {
    eprintln!("ERROR: {error}");
    ExitCode::FAILURE
}
