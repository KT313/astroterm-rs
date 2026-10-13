//! Step 5 of the frame (`project_stars_to_screen`, the warp of the region tiles onto the screen) beside the
//! projection geometry it and step 6 reuse: cameras, maps, arcs, the horizon and the view controls.
mod geometry;
mod forward;
mod warp;

pub use warp::{Frame, SkyArea, ViewKey, ViewTable};
pub use geometry::{
    ArcPart, CartesianCamera, Polar, ProjectedArc, ScreenPoint,
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_stereographic_horizontal, project_stereographic_north, project_to_cell, rotate_camera_into, zoom_view,
    project_constellation_segment, project_horizon_labels, project_horizon_line,
};

/// Step 5: project selected region images into the star layer. The forward prototype is selected in constants;
/// disabling it restores inverse sampling. Returns whether the inverse ray table was rebuilt (false for forward).
pub fn project_stars_to_screen(frame: &mut Frame, caches: &crate::regions::RegionCaches, selected: &crate::selection::Selected,
    observation: &crate::observation::Observation, view: &crate::model::View) -> bool {
    if crate::constants::FORWARD_TILE_PROJECTION {
        forward::project_tiles_forward(frame, caches, selected, observation, view);
        false
    } else {
        warp::project_tiles_inverse(frame, caches, selected, observation, view)
    }
}
