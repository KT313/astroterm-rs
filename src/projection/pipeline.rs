//! Projection stages in execution order; cache details and geometry live in private helper modules.
use crate::model::{ObservedSky, ProjectionViewport, View};
use crate::state::ProjectionCache;
use crate::timing::StepTimes;
use super::caching::{project_cached_stars, project_cached_draw_order, project_cached_bodies, project_cached_constellations, project_cached_horizon};

/// Refresh geometry and drawing order. Borrow the completed result with `borrow_projected` when needed.
pub fn project_cached_sky(storage: &mut ProjectionCache, sky: &ObservedSky, view: &View, viewport: ProjectionViewport, epoch: f64, times: &mut StepTimes) {
    storage.regional_active = false;
    let camera = times.measure("Camera preparation", || super::prepare_camera(view));       // prepare the current viewing direction and scale
    project_cached_stars(storage, sky, view, viewport, epoch, camera, times);                 // map visible stars to positions on the screen
    project_cached_draw_order(storage, sky, epoch, times);                                  // draw dim stars first so brighter stars remain visible
    project_cached_bodies(storage, sky, view, viewport, epoch, camera, times);                // position the Sun, planets and Moon
    project_cached_constellations(storage, sky, view, viewport, epoch, times);                // project constellation lines and clip them to the view
    project_cached_horizon(storage, view, viewport, epoch, times);                           // position the horizon and its direction labels
}

/// Regional entry point; the observation view keeps output and dependency versions borrowed together.
pub fn project_cached_regions(storage: &mut ProjectionCache, observed: crate::state::RegionalObservation<'_>, view: &View, viewport: ProjectionViewport, epoch: f64, times: &mut StepTimes) {
    let camera = times.measure("Camera preparation", || super::prepare_camera(view));       // prepare the current viewing direction and scale
    super::caching::project_regional_stars(storage, observed, view, viewport, epoch, camera, times); // update only regions whose inputs changed
    project_cached_bodies(storage, observed.sky(), view, viewport, epoch, camera, times);    // position the Sun, planets and Moon
    project_cached_constellations(storage, observed.sky(), view, viewport, epoch, times);    // project lines between constellation stars
    project_cached_horizon(storage, view, viewport, epoch, times);                          // position the horizon and direction labels
}
