//! astroterm: stars, planets, constellations and more, rendered in the terminal.

mod helpers;
mod pipeline;

use std::io;
use std::process::ExitCode;

use clap::Parser;

use astroterm::catalog::datasets::DatasetDirectories;
use astroterm::cli::Arguments;
use astroterm::constants::{SPEED_GROUP_BOUNDS, SPEED_GROUP_TTL_SECONDS};
use astroterm::model::{Config, Data};
use astroterm::time::{StartupTiming, StepTimes};

use helpers::{
    BenchmarkSession, Panel, begin_benchmark_capture, capture_display, finish_benchmark_report, finish_rendering, load_catalog, load_cities, open_display, open_frame_benchmark,
    print_bash_completions, validate_arguments,
};
use pipeline::run_frame_loop;

/// Parse options, build the sky data, and render until the user quits.
fn main() -> ExitCode {

    assert_stuff();

    let arguments = Arguments::parse();
    let capture_trace = arguments.debug_singleframe || arguments.debug_frames.is_some();
    let mut startup_timing = StartupTiming::new(capture_trace, arguments.debug_frametimes);             // capture startup timings when diagnostics are requested
    let Ok(cities) = load_cities(&mut startup_timing.step_times) else { return ExitCode::FAILURE; };     // load city data
    if arguments.bash_completions { return print_bash_completions(&cities); }                            // print shell completions and exit
    let Ok(config) = validate_arguments(arguments, &cities) else { return ExitCode::FAILURE; };          // validate config options
    if config.debug_log_data { eprintln!("--debug-log-data has no effect: the table dumps belonged to the retired frame pipeline."); }

    let Ok(mut benchmark) = open_frame_benchmark(&config) else { return ExitCode::FAILURE; };            // create a separate timing file only when requested
    let directories = DatasetDirectories::for_user();                                                    // locate dataset and cache on disk
    let Ok(mut data) = load_catalog(&config, &directories, &mut startup_timing.step_times) else { return ExitCode::FAILURE; }; // load the dataset and split it into the sky data

    let (result, step_times) = render_in_terminal(&config, &mut data, startup_timing, &mut benchmark);   // run frames within the terminal's cleanup scope
    let result = finish_benchmark_report(&mut benchmark, result);                                        // mark completion after terminal cleanup
    finish_rendering(result, &config, &step_times)                                                       // report results after the terminal is restored
}

/// Keep the display alive through the frame loop and restore the terminal before main prints the result.
fn render_in_terminal(config: &Config, data: &mut Data, mut startup_timing: StartupTiming, benchmark: &mut Option<BenchmarkSession>) -> (io::Result<()>, StepTimes) {
    // Prepare the display and drawing state before simulated time starts advancing.
    let mut display = match open_display(config, &mut startup_timing.step_times) {
        Ok(display) => display,
        Err(error) => return (Err(error), startup_timing.step_times),
    };
    data.star_opacities.prepare(1.0);
    let mut frame = display.new_frame();
    let mut panel = Panel::new(config, data);

    // Preserve startup measurements for the final report even if benchmark reporting fails.
    if let Err(error) = begin_benchmark_capture(benchmark, config, || capture_display(display.info()), &mut startup_timing.step_times) {
        return (Err(error), startup_timing.step_times);
    }

    // Only the initialized manager can enter the loop; return its measurements after rendering.
    let frame_step_seconds = config.frame_benchmark.as_ref().map(|benchmark| benchmark.step_seconds);
    let mut time_manager = startup_timing.start_simulation(
        config.simulation.start_julian_date, config.simulation.speed, config.fps, frame_step_seconds,
    );
    let result = run_frame_loop(config, data, &mut display, &mut frame, &mut panel, &mut time_manager, benchmark);
    (result, time_manager.step_times)
}

fn assert_stuff() {
    // Each speed group needs a TTL, including the group above the final speed threshold.
    assert_eq!(
        SPEED_GROUP_TTL_SECONDS.len(),
        SPEED_GROUP_BOUNDS.len() + 1,
        "SPEED_GROUP_TTL_SECONDS must have exactly one more entry than SPEED_GROUP_BOUNDS",
    );
}
