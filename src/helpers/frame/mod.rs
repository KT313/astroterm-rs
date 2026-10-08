//! Frame-stage operations over explicit inputs; the overall loop remains in pipeline.rs.
use std::io;
use astroterm::astro::{Observer, SimulationClock};
use astroterm::cache::CacheStats;
use astroterm::controls::apply_control;
use astroterm::model::{View, Sky, FrameTime, ProjectionViewport};
use astroterm::sky::update_solar_system;
use astroterm::state::{ObserverPreparationCache, ObservationCache, ProjectionCache, SimulationState, RenderingState};
use astroterm::terminal::{FrameInput, Renderer};
use astroterm::timing::StepTimes;
use super::record_projected_memory;

#[allow(clippy::too_many_arguments)]
/// Apply a frame's controls and invalidate dependent caches after any view change or resize.
/// The caller checks for quit first and retains ownership of the input until the frame ends.
pub(crate) fn apply_frame_controls(
    input: &FrameInput, initial_view: &View, view: &mut View, clock: &mut SimulationClock,
    renderer: &mut Renderer, rendering: &mut RenderingState, selection: &mut astroterm::state::StarSelectionCache, projection_cache: &mut ProjectionCache,
) -> io::Result<()> {
    let previous_view = *view;
    if input.resized { renderer.fit_to_terminal(rendering)?; }
    for &control in &input.controls {
        apply_control(control, view, clock, initial_view);
    }

    if *view != previous_view {
        selection.invalidate_view();
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

/// Prepare reception-time solar-system samples; catalog stars have a separate stage.
pub(crate) fn simulate_solar_system_frame(simulation_state: &mut SimulationState, time: FrameTime, step_times: &mut StepTimes) -> io::Result<()> {
    step_times
        .measure_steps("Solar-system simulation", |steps| {
            update_solar_system(simulation_state, time, &[], steps)
        })
        .map_err(io::Error::other)
}

/// Finish observer geometry and solar-system emission samples before selecting stars.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_observer_frame(
    site: Observer, simulation_state: &mut SimulationState, observer_cache: &mut ObserverPreparationCache,
    time: FrameTime, step_times: &mut StepTimes,
) -> io::Result<astroterm::model::ObserverState> {
    step_times.measure_steps("Observer preparation", |steps| {
        astroterm::sky::prepare_observer_inputs(observer_cache, simulation_state, time, site, steps)
    }).map_err(io::Error::other)
}

/// Select only conservative candidates and the endpoints needed by constellation lines.
pub(crate) fn select_stars_frame(storage: &mut astroterm::state::StarSelectionCache, catalog: &std::sync::Arc<astroterm::model::SkyCatalog>,
    observer: &astroterm::model::ObserverState, view: &View, threshold: f64, refraction: bool, times: &mut StepTimes) {
    times.measure_steps("Star selection", |times| astroterm::sky::select_cached_stars(storage, catalog, observer, threshold, refraction, astroterm::projection::select_view_region(view), times));
}

/// Update intrinsic directions and brightness without access to observer or camera data.
pub(crate) fn simulate_stars_frame(stars: &mut astroterm::state::StellarSimulationState, selection: astroterm::state::SelectedStars<'_>, time: FrameTime, times: &mut StepTimes) {
    times.measure_steps("Stellar simulation", |times| astroterm::sky::simulate_stars(stars, selection, time.tt, times));
}

/// Apply viewer-dependent corrections to the completed model results.
#[allow(clippy::too_many_arguments)]
pub(crate) fn observe_frame<'a>(
    observation: &'a mut ObservationCache, stars: astroterm::state::StellarResults<'_>, bodies: astroterm::state::PreparedBodies<'_>,
    observer: &astroterm::model::ObserverState, threshold: f64, refraction: bool, sky: &'a mut Sky, times: &mut StepTimes,
) -> astroterm::state::RegionalObservation<'a> {
    times.measure_steps("Observation", |times| astroterm::sky::observe_cached_regions(observation, stars, bodies, observer, threshold, refraction, sky, times))
}

/// Project the observed sky for the current viewport within the existing Projection timer.
pub(crate) fn project_frame(
    sky: astroterm::state::RegionalObservation<'_>, view: &View, viewport: astroterm::model::ProjectionViewport, time: FrameTime,
    projection_cache: &mut ProjectionCache, step_times: &mut StepTimes,
) {
    step_times.measure_steps("Projection", |steps| {
        astroterm::projection::project_cached_regions(projection_cache, sky, view, viewport, time.tt, steps);
    })
}

/// Publish cache statistics, borrow the completed geometry and render it without copying the projected data.
/// Existing assembly/raster/presentation timers stay inside their original operations.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_projected_frame(renderer: &mut Renderer, rendering: &mut RenderingState, sky: &Sky, view: &View,
    viewport: ProjectionViewport, projection: &ProjectionCache, observation_stats: CacheStats, utc: f64,
    clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<()> {
    renderer.set_cache_diagnostics(rendering, observation_stats, projection.stats()); // update the debug display's reuse counts
    let projected = borrow_frame_projection(sky, view, viewport, projection, times); // read completed geometry without copying it
    renderer.render_frame(rendering, &projected, view, utc, clock, observer, times)   // assemble the image and text, then present them
}

/// Borrow the completed projection once, then describe that same view outside the assembly timer.
fn borrow_frame_projection<'a>(sky: &'a Sky, view: &View, viewport: astroterm::model::ProjectionViewport,
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
