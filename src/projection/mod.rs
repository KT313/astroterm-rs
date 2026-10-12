//! Step 5 of the frame (`project_stars_to_screen`, the warp of the region tiles onto the screen) beside the
//! projection geometry it and step 6 reuse: cameras, maps, arcs, the horizon and the view controls.
mod geometry;
mod warp;

pub use warp::{Frame, SkyArea, ViewKey, ViewTable, project_stars_to_screen};
pub use geometry::{
    ArcPart, CartesianCamera, Polar, ProjectedArc, ScreenPoint,
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_stereographic_horizontal, project_stereographic_north, project_to_cell, rotate_camera_into, zoom_view,
    project_constellation_segment, project_horizon_labels, project_horizon_line,
};
