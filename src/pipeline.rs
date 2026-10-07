//! One-time frame preparation and the simulation → observation → projection → rendering loop.

use std::io;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use astroterm::astro::SimulationClock;
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;
use astroterm::state::{ApplicationState, StellarSimulationState};
use astroterm::model::SkyCatalog;

use crate::helpers::{
    apply_frame_controls, begin_frame_diagnostics, render_projected_frame, capture_memory, observe_frame, select_stars_frame, prepare_observer_frame, project_frame, finish_frame_diagnostics,
    stop_on_quit, resolve_frame_time, simulate_solar_system_frame, simulate_stars_frame, log_pipeline_data_if_requested,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_render_loop(state: &mut ApplicationState, renderer: &mut Renderer) -> io::Result<()> {

    log_pipeline_data_if_requested(state, "tmp/tables.md", "stage-renderloop-start")?;

    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(state.config.fps));                                // set the target time between frames from the requested FPS
    state.current_view = state.config.view;                                                                         // restore the configured starting view for this run

    renderer.configure_cache(&mut state.cache.rendering, &state.config.cache);

    prepare_frame_data(&state.cache.sky.catalog, &mut state.cache.simulation.stars, &mut state.timings)?;                // calculate values that stay the same for the whole run
    log_pipeline_data_if_requested(state, "tmp/after-preparation.md", "after-preparation")?;
    state.free_preparation_only_data();                                                                             // free startup-only movement bounds before frames begin
    log_pipeline_data_if_requested(state, "tmp/after-preparation-cleanup.md", "after-preparation-cleanup")?;
    capture_memory(&state.config, &state.persistent.catalog, &state.cache, state.preparation.as_ref(), renderer, &mut state.timings, "After preparation", None); // inspect loaded/prepared buffers only when requested
    let mut clock = SimulationClock::start(state.config.simulation.start_julian_date, state.config.simulation.speed); // start simulated time after setup is complete
    state.timings.reset_frame_timings();                                                                            // keep setup time out of the displayed frame timings

    loop {
        let frame_start = begin_frame_diagnostics(&mut state.timings);                                              // start this frame's timing and optional memory records

        let input = poll_frame_input(state.config.terminal.quit_on_any_key)?;                                       // read key presses and terminal size changes
        if stop_on_quit(&input, &mut state.timings) { return Ok(()); }                                              // stop on quit without processing other keys
        apply_frame_controls(&input, &state.config.view, &mut state.current_view, &mut clock, renderer, &mut state.cache.rendering, &mut state.cache.selection, &mut state.cache.projection)?; // apply controls and mark affected results for recalculation
        state.cache.simulation.solar_system.begin_frame();                                                                       // discard saved calculations where reuse is disabled
        let time = resolve_frame_time(state.config.debug_singleframe, state.config.simulation.start_julian_date, &clock); // choose the simulated date and time to display
        state.timings.set_memory_frame_time(time.utc, time.tt);                                                     // retain the chosen time even if a later stage fails

        simulate_solar_system_frame(&mut state.cache.simulation.solar_system, time, &mut state.timings)?;                                     // update planet and Moon positions and Earth's axis direction as needed
        let observer = prepare_observer_frame(state.config.simulation.observer, &mut state.cache.simulation.solar_system, &mut state.cache.observer, time, &mut state.timings)?; // finish observer geometry and solar-system light-time inputs
        select_stars_frame(&mut state.cache.selection, &state.persistent.catalog, &observer, &state.current_view, state.config.render.magnitude_threshold, state.config.simulation.refraction, &mut state.timings); // select potentially visible stars and constellation endpoints
        simulate_stars_frame(&mut state.cache.simulation.stars, state.cache.selection.stars(), time, &mut state.timings); // calculate motion and brightness only for selected stars
        observe_frame(&mut state.cache.observation, state.cache.simulation.stars.results(state.cache.selection.stars()), state.cache.observer.bodies(&observer), &observer, state.config.render.magnitude_threshold, state.config.simulation.refraction, &mut state.cache.sky, &mut state.timings); // calculate how prepared objects appear to the viewer

        let viewport = renderer.viewport(&state.cache.rendering);                                                   // use the current terminal size after any resize was handled
        project_frame(&state.cache.sky, &state.current_view, viewport, time, &mut state.cache.projection, &mut state.timings); // convert visible sky positions into positions on the screen
        log_pipeline_data_if_requested(state, "tmp/after-projection.md", "after-projection")?;

        let sky_processing_stats = state.cache.sky_processing_stats();                                             // summarize all sky-processing caches for the metadata
        render_projected_frame(renderer, &mut state.cache.rendering, &state.cache.sky, &state.current_view, viewport, &state.cache.projection, sky_processing_stats, time.utc, &clock, &state.config.simulation.observer, &mut state.timings)?; // draw and present the projected sky and text
        log_pipeline_data_if_requested(state, "tmp/after-rendering.md", "after-rendering")?;

        finish_frame_diagnostics(&state.config, &state.persistent.catalog, &state.cache, state.preparation.as_ref(), renderer, frame_start, time, &mut state.timings); // save frame timings and requested memory diagnostics

        if state.config.debug_singleframe { return Ok(()); }                                                        // stop once the requested single-frame diagnostics are captured
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                        // wait until the next frame is due, unless already running late
    }
}

/// Prepare values that depend only on this run's immutable catalog. Time and camera results stay in the loop.
fn prepare_frame_data(catalog: &Arc<SkyCatalog>, stars: &mut StellarSimulationState, times: &mut StepTimes) -> io::Result<()> {
    catalog.validate_exception_support()?;                                                                          // reject unsupported sparse data before the frame loop
    times.measure_steps("Frame preparation", |times| {
        astroterm::sky::prepare_stellar_catalog(stars, catalog.clone(), times);                           // record how each star's position and brightness can change
    });
    Ok(())
}
