//! Step 5 of the frame (`project_stars_to_screen`, the warp of the region tiles onto the screen) beside the
//! projection geometry it and step 6 reuse: cameras, maps, arcs, the horizon and the view controls.
use crate::time::StepTimes;
mod geometry;
mod raster;
mod tile_warp;
mod coons_warp;
mod hermite_warp;
mod adaptive_warp;
mod patch_warp;
mod triangle_rows;
mod tile_sampling;
mod frame;

pub use frame::Frame;
pub use geometry::{
    ArcPart, CartesianCamera, Polar, ProjectedArc, ScreenPoint,
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_stereographic_horizontal, project_stereographic_north, project_to_cell, rotate_camera_into, zoom_view,
    project_constellation_segment, project_horizon_labels, project_horizon_line,
};

/// Step 5: return a square image of projected stars, with no frame offsets or terminal margins.
/// Pixels use row-major order and retain floating-point RGB and opacity until final presentation.
pub fn project_stars_to_screen(
    drawing_area_width_and_height: usize,
    data: &crate::model::Data,
    selected: &crate::selection::Selected,
    observation: &crate::observation::Observation,
    view: &crate::model::View,
    times: &mut StepTimes
) -> Vec<crate::model::Texel> {
    // Render in square-local coordinates; the caller places the result into the full frame.
    adaptive_warp::project_adaptive_tiles(drawing_area_width_and_height, data, selected, observation, view, times)
}
