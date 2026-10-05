//! astroterm: stars, planets, constellations and more, rendered in the terminal.

mod helpers;
mod pipeline;

use std::process::ExitCode;

use clap::Parser;

use astroterm::catalog::datasets::DatasetDirectories;
use astroterm::cli::Arguments;
use astroterm::state::ApplicationState;

use helpers::{
    finish_rendering, load_catalog_sky, load_cities, print_bash_completions, render_in_terminal, start_step_times, validate_arguments,
};

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {

    let arguments = Arguments::parse();
    let mut step_times = start_step_times(&arguments);                                        // capture startup timings when diagnostics are requested
    let Ok(cities) = load_cities(&mut step_times) else { return ExitCode::FAILURE; };           // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                   // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; }; // validate config options

    #[cfg(feature = "memory-diagnostics")]
    if config.debug_memory { step_times.enable_memory_run(config.cache.enabled); }            // retain bounded startup and frame diagnostics when requested

    let directories = DatasetDirectories::for_user();                                           // locate dataset and cache on disk
    let Ok(sky) = load_catalog_sky(&config, &directories, &mut step_times) else { return ExitCode::FAILURE; }; // load dataset

    let mut state = ApplicationState::new(config, sky, step_times);                           // group validated settings and loaded working data
    let result = render_in_terminal(&mut state);                                              // run frames within the terminal's cleanup scope
    finish_rendering(result, &state)                                                          // report results after the terminal is restored
}
