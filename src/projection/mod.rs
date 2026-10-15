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

pub use frame::{Frame, SkyArea};
pub use geometry::{
    ArcPart, CartesianCamera, Polar, ProjectedArc, ScreenPoint,
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_stereographic_horizontal, project_stereographic_north, project_to_cell, rotate_camera_into, zoom_view,
    project_constellation_segment, project_horizon_labels, project_horizon_line,
};

/// Step 5: forward-map merged tiles onto the screen. The selected backend owns projection and rasterization.
pub fn project_stars_to_screen(
    frame: &mut Frame, 
    data: &crate::model::Data, 
    selected: &crate::selection::Selected,
    observation: &crate::observation::Observation, 
    view: &crate::model::View, 
    times: &mut StepTimes
) {
    // Map the whole region tiles onto the frame's star layer.
    adaptive_warp::project_adaptive_tiles(frame, data, selected, observation, view, times);
}
