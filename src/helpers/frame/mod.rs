//! Frame-stage operations over explicit inputs; the overall loop remains in pipeline.rs.
use std::io;
use astroterm::astro::SimulationClock;
use astroterm::controls::apply_control;
use astroterm::model::{View, Sky, FrameTime, SimulationSettings};
use astroterm::sky::update_simulation;
use astroterm::state::{ObservationCache, ProjectionCache, SimulationState, RenderingState};
use astroterm::terminal::{FrameInput, Renderer};
use astroterm::timing::StepTimes;
use super::{describe_observer_geometry, describe_light_time_sampling, record_projected_memory};

#[allow(clippy::too_many_arguments)]
/// Apply a frame's controls and invalidate dependent caches after any view change or resize.
/// The caller checks for quit first and retains ownership of the input until the frame ends.
pub(crate) fn apply_frame_controls(
    input: &FrameInput, initial_view: &View, view: &mut View, clock: &mut SimulationClock,
    renderer: &mut Renderer, rendering: &mut RenderingState, observation_cache: &mut ObservationCache, projection_cache: &mut ProjectionCache,
) -> io::Result<()> {
    let previous_view = *view;
    if input.resized { renderer.fit_to_terminal(rendering)?; }
    for &control in &input.controls {
        apply_control(control, view, clock, initial_view);
    }

    if *view != previous_view {
        observation_cache.invalidate_view();
        projection_cache.invalidate_view();
    } else if input.resized {
        projection_cache.invalidate_view();
    }
    Ok(())
}

/// Use the exact requested epoch for single-frame diagnostics, otherwise read the running clock.
pub(crate) fn resolve_frame_time(single_frame: bool, start_julian_date: f64, clock: &SimulationClock) -> FrameTime {
    FrameTime::from_utc(if single_frame {
        start_julian_date
    } else {
        clock.julian_date()
    })
}

/// Refresh model samples within the existing Simulation timer and preserve its error conversion.
pub(crate) fn simulate_frame(simulation_state: &mut SimulationState, time: FrameTime, step_times: &mut StepTimes) -> io::Result<()> {
    step_times
        .measure_steps("Simulation", |steps| {
            update_simulation(simulation_state, time, &[], steps)
        })
        .map_err(io::Error::other)
}

/// Prepare observer/emission samples and run the ordered observation passes within their existing timers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn observe_frame(
    simulation: &SimulationSettings, magnitude_threshold: f64, view: &View, simulation_state: &mut SimulationState,
    observation_cache: &mut ObservationCache, sky: &mut Sky, time: FrameTime, step_times: &mut StepTimes,
) -> io::Result<()> {
    step_times.measure_steps("Observation", |steps| {
        let mut observer = astroterm::sky::prepare_cached_observer_with_times(observation_cache, simulation_state, time, simulation.observer, steps)?;
        describe_observer_geometry(time, &simulation.observer, steps);
        steps.measure_steps("Light-time sampling", |steps| {
            astroterm::sky::prepare_cached_light_time(observation_cache, simulation_state, &mut observer, steps)
        })?;

        describe_light_time_sampling(&observer, observation_cache, steps);

        astroterm::sky::observe_cached_sky(observation_cache, simulation_state, &observer, magnitude_threshold,
            simulation.refraction, astroterm::projection::select_view_region(view), sky, steps) // filter stars and calculate their apparent directions
    }).map_err(io::Error::other)
}

/// Project the observed sky for the current viewport within the existing Projection timer.
pub(crate) fn project_frame(
    sky: &Sky, view: &View, viewport: astroterm::model::ProjectionViewport, time: FrameTime,
    projection_cache: &mut ProjectionCache, step_times: &mut StepTimes,
) {
    step_times.measure_steps("Projection", |steps| {
        astroterm::projection::project_cached_sky(projection_cache, sky, view, viewport, time.tt, steps);
    })
}

/// Borrow the completed projection once, then describe that same view outside the assembly timer.
pub(crate) fn borrow_frame_projection<'a>(sky: &'a Sky, view: &View, viewport: astroterm::model::ProjectionViewport,
    projection_cache: &'a ProjectionCache, times: &mut StepTimes) -> astroterm::model::ProjectedSky<'a> {
    let projected = times.measure("Projected view assembly", || astroterm::projection::borrow_projected(projection_cache, sky, view, viewport));
    record_projected_memory(times, &projected);
    times.describe("Projected view assembly", || format!("ordered stars={}; projected-reference records allocated=0; geometry borrowed", projected.stars.len()));
    projected
}

/// Cancel the empty diagnostic frame on a quit key without losing the last successfully presented frame.
pub(crate) fn stop_on_quit(input: &FrameInput, times: &mut StepTimes) -> bool {
    if !input.controls.contains(&astroterm::controls::Control::Quit) { return false; }
    times.cancel_memory_frame();
    true
}
