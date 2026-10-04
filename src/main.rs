//! astroterm: stars, planets, constellations and more, rendered in the terminal.

mod helpers;
mod pipeline;

use std::process::ExitCode;

use clap::Parser;

use astroterm::catalog::datasets::DatasetDirectories;
use astroterm::cli::Arguments;
use astroterm::timing::StepTimes;

use helpers::{
    finish_rendering, load_catalog_sky, load_cities, print_bash_completions, render_in_terminal, validate_arguments,
};

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {

    let arguments = Arguments::parse();
    let mut step_times = StepTimes::with_trace(arguments.debug_singleframe);                    // init tracker for performance analysis
    let Ok(cities) = load_cities(&mut step_times) else { return ExitCode::FAILURE; };           // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                   // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; }; // validate config options

    let directories = DatasetDirectories::for_user();                                           // locate dataset and cache on disk
    let Ok(mut sky) = load_catalog_sky(&config, &directories, &mut step_times) else { return ExitCode::FAILURE; }; // load dataset

    let result = render_in_terminal(&config, &mut sky, &mut step_times);                        // render loop
    finish_rendering(result, &config, &step_times)                                              // report result and return exit status
}
