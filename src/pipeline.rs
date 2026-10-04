//! One-time frame preparation and the simulation → observation → projection → rendering loop.

use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use astroterm::astro::SimulationClock;
use astroterm::cli::Config;
use astroterm::controls::{Control, apply_control};
use astroterm::projection::ProjectionCache;
use astroterm::sky::{FrameTime, ObservationCache, SimulationState, Sky, update_simulation};
use astroterm::terminal::{Renderer, poll_frame_input};
use astroterm::timing::StepTimes;

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_render_loop(
    config: &Config,
    sky: &mut Sky,
    renderer: &mut Renderer,
    step_times: &mut StepTimes,
) -> io::Result<()> {
    // start from the configured view and time
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));
    let mut view = config.view;
    let simulation = &config.simulation;

    let mut simulation_state = SimulationState::default();
    simulation_state.configure_cache(&config.cache);
    let mut observation_cache = ObservationCache::new(config.cache.clone());
    let mut projection_cache = ProjectionCache::new(config.cache.clone());
    renderer.configure_cache(&config.cache);

    // prepare immutable catalog-derived inputs before starting the simulation clock and frame loop
    prepare_frame_data(
        &sky.catalog,
        &mut observation_cache,
        &mut projection_cache,
        renderer,
        step_times,
    );
    let mut clock = SimulationClock::start(simulation.start_julian_date, simulation.speed);
    step_times.reset_frame_timings();

    loop {
        let frame_start = Instant::now();
        step_times.begin_frame();

        // handle key presses and terminal resizes
        let input = poll_frame_input(config.terminal.quit_on_any_key)?;
        if input.controls.contains(&Control::Quit) {
            return Ok(());
        }
        let previous_view = view;
        if input.resized {
            renderer.fit_to_terminal()?;
        }
        for &control in &input.controls {
            apply_control(control, &mut view, &mut clock, &config.view);
        }

        if view != previous_view {
            observation_cache.invalidate_view();
            projection_cache.invalidate_view();
        } else if input.resized {
            projection_cache.invalidate_view();
        }
        simulation_state.begin_frame();

        // refresh only model samples whose validity no longer covers this frame
        let time = FrameTime::from_utc(if config.debug_singleframe {
            simulation.start_julian_date // exact requested epoch makes single-frame comparisons reproducible
        } else {
            clock.julian_date()
        });
        step_times
            .measure_steps("Simulation", |steps| {
                update_simulation(&mut simulation_state, time, &[], steps)
            })
            .map_err(io::Error::other)?;

        // observe at the current epoch, with exact body spin and observer corrections
        step_times
            .measure_steps("Observation", |steps| {
                let mut observer = steps.measure("Observer geometry", || {
                    observation_cache.prepare_observer(&simulation_state, time, simulation.observer)
                })?;
                steps.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, simulation.observer.latitude, simulation.observer.longitude));
                steps.measure_steps("Light-time sampling", |steps| {
                    observation_cache.prepare_light_time(&mut simulation_state, &mut observer, steps)
                })?;

                steps.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));

                for name in ["Observer geometry", "Light-time sampling"] {
                    steps.describe(name, || format!("cache={:?}", observation_cache.reports().into_iter().find(|r| r.name == name).unwrap()));
                }

                // observation.rs owns the timed filtering, motion, aberration, rotation and refraction passes
                observation_cache.observe(
                    &simulation_state,
                    &observer,
                    config.render.magnitude_threshold,
                    simulation.refraction,
                    view.sky_region(),
                    sky,
                    steps,
                )
            })
            .map_err(io::Error::other)?;

        // project the immutable observed sky for this camera, then render prepared screen geometry
        let projected = step_times.measure_steps("Projection", |steps| {
            projection_cache.project(sky, &view, renderer.viewport(), time.tt, steps)
        });
        renderer.set_cache_diagnostics(observation_cache.stats(), projection_cache.stats());
        renderer.render_frame(&projected, &view, time.utc, &clock, &simulation.observer, step_times)?;

        if config.debug_singleframe {
            step_times.describe("Present", || {
                format!(
                    "frame elapsed including diagnostic bookkeeping={:.3} ms; UTC JD={:.9}; sleep excluded",
                    frame_start.elapsed().as_secs_f64() * 1000.0,
                    time.utc
                )
            });
            return Ok(());
        }

        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed())); // wait for the rest of the frame
    }
}

/// Prepare values that depend only on this run's immutable catalog. Time and camera results stay in the loop.
fn prepare_frame_data(
    catalog: &Arc<astroterm::sky::SkyCatalog>,
    observation: &mut ObservationCache,
    projection: &mut ProjectionCache,
    renderer: &mut Renderer,
    times: &mut StepTimes,
) {
    times.measure_steps("Frame preparation", |times| {
        observation.prepare_catalog(catalog.clone(), times);
        projection.prepare_catalog(catalog, times);
        renderer.prepare_catalog(catalog.clone(), times);
    });
}
