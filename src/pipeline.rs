//! One-time frame preparation and the simulation → observation → projection → rendering loop.

use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use astroterm::astro::SimulationClock;
use astroterm::cli::Config;
use astroterm::controls::Control;
use astroterm::projection::ProjectionCache;
use astroterm::sky::{ObservationCache, SimulationState, Sky, SkyCatalog};
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;

use crate::helpers::{apply_frame_controls, observe_frame, project_frame, record_single_frame_if_requested, resolve_frame_time, simulate_frame};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_render_loop(config: &Config, sky: &mut Sky, renderer: &mut Renderer, step_times: &mut StepTimes) -> io::Result<()> {

    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps)); // set the target time between frames from the requested FPS
    let mut view = config.view;
    let simulation = &config.simulation;

    let mut simulation_state = SimulationState::default();                     // create storage for calculated positions and reusable results
    simulation_state.configure_cache(&config.cache);
    let mut observation_cache = ObservationCache::new(config.cache.clone());
    let mut projection_cache = ProjectionCache::new(config.cache.clone());
    renderer.configure_cache(&config.cache);

    prepare_frame_data(&sky.catalog, &mut observation_cache, &mut projection_cache, renderer, step_times); // calculate values that stay the same for the whole run
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);                // start simulated time after setup is complete
    step_times.reset_frame_timings();                                                                      // keep setup time out of the displayed frame timings

    loop {
        let frame_start = Instant::now();
        step_times.begin_frame();

        let input = poll_frame_input(config.terminal.quit_on_any_key)?;                                    // read key presses and terminal size changes
        if input.controls.contains(&Control::Quit) { return Ok(()); }                                      // stop on quit without processing other keys
        apply_frame_controls(&input, config, &mut view, &mut clock, renderer, &mut observation_cache, &mut projection_cache)?; // apply controls and mark affected results for recalculation
        simulation_state.begin_frame();                                                                    // discard saved calculations where reuse is disabled

        let time = resolve_frame_time(config, &clock);                                                     // choose the simulated date and time to display
        simulate_frame(&mut simulation_state, time, step_times)?;                                          // update planet and Moon positions and Earth's axis direction as needed
        observe_frame(config, &view, &mut simulation_state, &mut observation_cache, sky, time, step_times)?; // calculate where objects appear from the viewer's location
        let projected = project_frame(sky, &view, renderer, time, &mut projection_cache, step_times);      // convert visible sky positions into positions on the screen
        renderer.set_cache_diagnostics(observation_cache.stats(), projection_cache.stats());               // pass result-reuse counts to the debug display
        renderer.render_frame(&projected, &view, time.utc, &clock, &simulation.observer, step_times)?;     // draw the sky and text, then display them in the terminal

        if record_single_frame_if_requested(config, frame_start, time, step_times) { return Ok(()); }      // record timings and stop when --debug-singleframe is set
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                               // wait until the next frame is due, unless already running late
    }
}

/// Prepare values that depend only on this run's immutable catalog. Time and camera results stay in the loop.
fn prepare_frame_data(catalog: &Arc<SkyCatalog>, observation: &mut ObservationCache, projection: &mut ProjectionCache, renderer: &mut Renderer, times: &mut StepTimes) {
    times.measure_steps("Frame preparation", |times| {
        observation.prepare_catalog(catalog.clone(), times); // record how each star's position and brightness can change
        projection.prepare_catalog(catalog, times);          // store which stars each constellation line connects
        renderer.prepare_catalog(catalog.clone(), times);    // store each star's color and whether it has a name
    });
}
