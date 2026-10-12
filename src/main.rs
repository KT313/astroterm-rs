//! astroterm: stars, planets, constellations and more, rendered in the terminal.

mod helpers;
mod pipeline;

use std::io;
use std::process::ExitCode;

use clap::Parser;

use astroterm::catalog::datasets::DatasetDirectories;
use astroterm::cli::Arguments;
use astroterm::state::ApplicationState;

use helpers::{
    finish_rendering, load_catalog, load_cities,
    prepare_terminal, print_bash_completions, start_step_times, validate_arguments, BenchmarkSession, open_frame_benchmark, finish_benchmark_report,
};
use pipeline::run_render_loop;

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {

    let arguments = Arguments::parse();
    let mut step_times = start_step_times(&arguments);                                          // capture startup timings when diagnostics are requested
    let Ok(cities) = load_cities(&mut step_times) else { return ExitCode::FAILURE; };           // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                   // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; }; // validate config options

    let mut state = ApplicationState::new(config, step_times);                                  // create every working-data owner before any data is loaded
    let Ok(mut benchmark) = open_frame_benchmark(&state.config) else { return ExitCode::FAILURE; }; // create a separate timing file only when requested
    let directories = DatasetDirectories::for_user();                                           // locate dataset and cache on disk
    let Ok(()) = load_catalog(&mut state, &directories) else { return ExitCode::FAILURE; };    // load and prepare the dataset into the state

    let result = render_in_terminal(&mut state, &mut benchmark);                            // run frames within the terminal's cleanup scope
    let result = finish_benchmark_report(&mut benchmark, result);                              // mark completion after terminal cleanup
    finish_rendering(result, &state)                                                            // report results after the terminal is restored
}

/// Keep the renderer alive through the frame loop and restore the terminal before main prints the result.
fn render_in_terminal(state: &mut ApplicationState, benchmark: &mut Option<BenchmarkSession>) -> io::Result<()> {
    let mut renderer = prepare_terminal(state)?;         // open the terminal and prepare its rendering buffers
    run_render_loop(state, &mut renderer, benchmark)     // run frames until the requested limit, quit or an error; the renderer restores the terminal as this scope ends
}
