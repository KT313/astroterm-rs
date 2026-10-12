//! astroterm: stars, planets, constellations and more, rendered in the terminal.

mod helpers;
mod pipeline;

use std::io;
use std::process::ExitCode;

use clap::Parser;

use astroterm::catalog::datasets::DatasetDirectories;
use astroterm::cli::Arguments;
use astroterm::model::{Config, SkyTables};
use astroterm::timing::StepTimes;

use helpers::{
    BenchmarkSession, finish_benchmark_report, finish_rendering, load_catalog, load_cities, open_display, open_frame_benchmark,
    print_bash_completions, start_step_times, validate_arguments,
};
use pipeline::run_frame_loop;

/// Parse options, build the sky tables, and render until the user quits.
fn main() -> ExitCode {

    let arguments = Arguments::parse();
    let mut times = start_step_times(&arguments);                                                   // capture startup timings when diagnostics are requested
    let Ok(cities) = load_cities(&mut times) else { return ExitCode::FAILURE; };                    // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                       // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; };     // validate config options
    if config.debug_log_data { eprintln!("--debug-log-data has no effect: the table dumps belonged to the retired frame pipeline."); }

    let Ok(mut benchmark) = open_frame_benchmark(&config) else { return ExitCode::FAILURE; };       // create a separate timing file only when requested
    let directories = DatasetDirectories::for_user();                                               // locate dataset and cache on disk
    let Ok(tables) = load_catalog(&config, &directories, &mut times) else { return ExitCode::FAILURE; }; // load the dataset and split it into the sky tables

    let result = render_in_terminal(&config, &tables, &mut times, &mut benchmark);                  // run frames within the terminal's cleanup scope
    let result = finish_benchmark_report(&mut benchmark, result);                                   // mark completion after terminal cleanup
    finish_rendering(result, &config, &times)                                                       // report results after the terminal is restored
}

/// Keep the display alive through the frame loop and restore the terminal before main prints the result.
fn render_in_terminal(config: &Config, tables: &SkyTables, times: &mut StepTimes, benchmark: &mut Option<BenchmarkSession>) -> io::Result<()> {
    let mut display = open_display(config, times)?;             // open the terminal and detect its graphics protocol
    run_frame_loop(config, tables, &mut display, times, benchmark) // run frames until the requested limit, quit or an error; the display restores the terminal as this scope ends
}
