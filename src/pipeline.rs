//! One-time frame preparation and the simulation → observation → projection → rendering loop.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use astroterm::astro::SimulationClock;
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;
use astroterm::state::{ApplicationState, Caches};

use crate::helpers::{
    apply_frame_controls, borrow_frame_projection, observe_frame, project_frame, record_frame_duration,
    stop_on_quit, resolve_frame_time, simulate_frame, log_pipeline_data,
};
use crate::helpers::capture_memory;

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_render_loop(state: &mut ApplicationState, renderer: &mut Renderer) -> io::Result<()> {

    if state.config.debug_log_data { log_pipeline_data(state, "tmp/tables.md", "stage-renderloop-start")?; }

    let config = &state.config;

    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));                                   // set the target time between frames from the requested FPS
    let mut view = config.view;
    let simulation = &config.simulation;

    renderer.configure_cache(&mut state.cache.rendering, &config.cache);

    prepare_frame_data(&mut state.cache, renderer, &mut state.timings);                                          // calculate values that stay the same for the whole run
    if config.debug_log_data { log_pipeline_data(state, "tmp/after-preparation.md", "after-preparation")?; }
    capture_memory(config, &state.persistent.catalog, &state.cache, renderer, &mut state.timings, "After preparation", None); // inspect loaded/prepared buffers only when requested
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);                      // start simulated time after setup is complete
    state.timings.reset_frame_timings();                                                                         // keep setup time out of the displayed frame timings

    loop {
        let frame_start = Instant::now();
        state.timings.begin_frame();
        state.timings.begin_memory_frame();                                                                      // keep this frame separate from startup and the last completed frame

        let input = poll_frame_input(config.terminal.quit_on_any_key)?;                                          // read key presses and terminal size changes
        if stop_on_quit(&input, &mut state.timings) { return Ok(()); }                                           // stop on quit without processing other keys
        apply_frame_controls(&input, config, &mut view, &mut clock, renderer, &mut state.cache.rendering, &mut state.cache.observation, &mut state.cache.projection)?; // apply controls and mark affected results for recalculation
        state.cache.simulation.begin_frame();                                                                    // discard saved calculations where reuse is disabled

        let time = resolve_frame_time(config, &clock);                                                           // choose the simulated date and time to display
        state.timings.set_memory_frame_time(time.utc, time.tt);                                                  // retain the chosen time even if a later stage fails
        simulate_frame(&mut state.cache.simulation, time, &mut state.timings)?;                                  // update planet and Moon positions and Earth's axis direction as needed
        observe_frame(config, &view, &mut state.cache.simulation, &mut state.cache.observation, &mut state.cache.sky, time, &mut state.timings)?; // calculate where objects appear from the viewer's location
        project_frame(&state.cache.sky, &view, renderer.viewport(&state.cache.rendering), time, &mut state.cache.projection, &mut state.timings); // convert visible sky positions into positions on the screen
        if config.debug_log_data { log_pipeline_data(state, "tmp/after-projection.md", "after-projection")?; }
        renderer.set_cache_diagnostics(&mut state.cache.rendering, state.cache.observation.stats(), state.cache.projection.stats()); // pass result-reuse counts to the debug display
        let projected = borrow_frame_projection(&state.cache.sky, &view, renderer.viewport(&state.cache.rendering), &state.cache.projection, &mut state.timings); // read completed screen positions without copying them
        renderer.render_frame(&mut state.cache.rendering, &projected, &view, time.utc, &clock, &simulation.observer, &mut state.timings)?; // draw the sky and text, then display them in the terminal
        if config.debug_log_data { log_pipeline_data(state, "tmp/after-rendering.md", "after-rendering")?; }

        let elapsed = record_frame_duration(config, frame_start, time, &mut state.timings);                      // record frame duration before the final memory inspection
        capture_memory(config, &state.persistent.catalog, &state.cache, renderer, &mut state.timings, "After presented frame", Some(time.tt));

        state.timings.complete_memory_frame(elapsed);                                                            // retain completed diagnostics only after successful presentation

        if config.debug_singleframe { return Ok(()); }                                                           // stop once the requested single-frame diagnostics are captured
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                     // wait until the next frame is due, unless already running late
    }
}

/// Prepare values that depend only on this run's immutable catalog. Time and camera results stay in the loop.
fn prepare_frame_data(cache: &mut Caches, renderer: &mut Renderer, times: &mut StepTimes) {
    let Caches { sky, observation, projection, rendering, .. } = cache;
    let catalog = &sky.catalog;
    times.measure_steps("Frame preparation", |times| {
        astroterm::sky::prepare_observation_catalog(observation, catalog.clone(), times);                        // record how each star's position and brightness can change
        astroterm::projection::prepare_projection_catalog(projection, catalog, times);                           // store which stars each constellation line connects
        renderer.prepare_catalog(rendering, catalog.clone(), times);                                             // store each star's color and whether it has a name
    });
}
