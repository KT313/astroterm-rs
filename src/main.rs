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
    capture_failed_frame_memory, configure_memory_reporting, finish_rendering, load_catalog_sky, load_cities,
    prepare_terminal, print_bash_completions, start_step_times, validate_arguments,
};
use pipeline::run_render_loop;

/// Parse options, build the sky, and render it until the user quits.
fn main() -> ExitCode {

    let arguments = Arguments::parse();
    let mut step_times = start_step_times(&arguments);                                          // capture startup timings when diagnostics are requested
    let Ok(cities) = load_cities(&mut step_times) else { return ExitCode::FAILURE; };           // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                   // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; }; // validate config options

    configure_memory_reporting(&config, &mut step_times);                                       // retain startup and frame diagnostics when requested

    let directories = DatasetDirectories::for_user();                                           // locate dataset and cache on disk
    let Ok(sky) = load_catalog_sky(&config, &directories, &mut step_times) else { return ExitCode::FAILURE; }; // load dataset

    let mut state = ApplicationState::new(config, sky, step_times);                             // group validated settings and loaded working data
    let result = render_in_terminal(&mut state);                                                // run frames within the terminal's cleanup scope
    finish_rendering(result, &state)                                                            // report results after the terminal is restored
}

/// Keep the renderer alive through the frame loop and restore the terminal before main prints the result.
fn render_in_terminal(state: &mut ApplicationState) -> io::Result<()> {
    let mut renderer = prepare_terminal(state)?;                  // open the terminal and prepare its rendering buffers
    let result = run_render_loop(state, &mut renderer);           // run frames until quit, single-frame completion or an error
    capture_failed_frame_memory(state, &renderer, &result);       // save partial diagnostics while the renderer is still available
    result                                                        // the renderer restores the terminal as this scope ends
}
