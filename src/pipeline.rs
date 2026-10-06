//! One-time frame preparation and the simulation → observation → projection → rendering loop.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use astroterm::astro::SimulationClock;
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;
use astroterm::state::{ApplicationState, RunState};

use crate::helpers::{
    apply_frame_controls, borrow_frame_projection, observe_frame, project_frame, record_frame_duration,
    stop_on_quit, resolve_frame_time, simulate_frame,
};
use crate::helpers::capture_memory;

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_render_loop(state: &mut ApplicationState, renderer: &mut Renderer) -> io::Result<()> {

    let ApplicationState { config, catalog, run, timings: step_times } = state;

    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));                            // set the target time between frames from the requested FPS
    let mut view = config.view;
    let simulation = &config.simulation;

    renderer.configure_cache(&mut run.rendering, &config.cache);

    prepare_frame_data(run, renderer, step_times);                                                        // calculate values that stay the same for the whole run
    capture_memory(config, catalog, run, renderer, step_times, "After preparation", None);                // inspect loaded/prepared buffers only when requested
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);               // start simulated time after setup is complete
    step_times.reset_frame_timings();                                                                     // keep setup time out of the displayed frame timings

    loop {
        let frame_start = Instant::now();
        let RunState { sky, simulation: simulation_state, observation: observation_cache, projection: projection_cache, rendering } = &mut *run;
        step_times.begin_frame();
        step_times.begin_memory_frame();                                                                  // keep this frame separate from startup and the last completed frame

        let input = poll_frame_input(config.terminal.quit_on_any_key)?;                                   // read key presses and terminal size changes
        if stop_on_quit(&input, step_times) { return Ok(()); }                                            // stop on quit without processing other keys
        apply_frame_controls(&input, config, &mut view, &mut clock, renderer, rendering, observation_cache, projection_cache)?;  // apply controls and mark affected results for recalculation
        simulation_state.begin_frame();                                                                   // discard saved calculations where reuse is disabled

        let time = resolve_frame_time(config, &clock);                                                    // choose the simulated date and time to display
        step_times.set_memory_frame_time(time.utc, time.tt);                                              // retain the chosen time even if a later stage fails
        simulate_frame(simulation_state, time, step_times)?;                                              // update planet and Moon positions and Earth's axis direction as needed
        observe_frame(config, &view, simulation_state, observation_cache, sky, time, step_times)?;        // calculate where objects appear from the viewer's location
        project_frame(sky, &view, renderer.viewport(rendering), time, projection_cache, step_times);      // convert visible sky positions into positions on the screen
        renderer.set_cache_diagnostics(rendering, observation_cache.stats(), projection_cache.stats());   // pass result-reuse counts to the debug display
        let projected = borrow_frame_projection(sky, &view, renderer.viewport(rendering), projection_cache, step_times);  // read completed screen positions without copying them
        renderer.render_frame(rendering, &projected, &view, time.utc, &clock, &simulation.observer, step_times)?;  // draw the sky and text, then display them in the terminal

        let elapsed = record_frame_duration(config, frame_start, time, step_times);                       // record frame duration before the final memory inspection
        capture_memory(config, catalog, run, renderer, step_times, "After presented frame", Some(time.tt));

        step_times.complete_memory_frame(elapsed);                                                        // retain completed diagnostics only after successful presentation

        if config.debug_singleframe { return Ok(()); }                                                    // stop once the requested single-frame diagnostics are captured
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                              // wait until the next frame is due, unless already running late
    }
}

/// Prepare values that depend only on this run's immutable catalog. Time and camera results stay in the loop.
fn prepare_frame_data(run: &mut RunState, renderer: &mut Renderer, times: &mut StepTimes) {
    let RunState { sky, observation, projection, rendering, .. } = run;
    let catalog = &sky.catalog;
    times.measure_steps("Frame preparation", |times| {
        astroterm::sky::prepare_observation_catalog(observation, catalog.clone(), times);                 // record how each star's position and brightness can change
        astroterm::projection::prepare_projection_catalog(projection, catalog, times);                    // store which stars each constellation line connects
        renderer.prepare_catalog(rendering, catalog.clone(), times);                                      // store each star's color and whether it has a name
    });
}
