//! Step 5 of the frame (`project_stars_to_screen`, the warp of the region tiles onto the screen) beside the
//! projection geometry it and step 6 reuse: cameras, maps, arcs, the horizon and the view controls.
use crate::time::StepTimes;
mod geometry;
mod pixel_warp;
mod region_drawing;
mod executor;
mod worker_images;
mod worker_image;
mod screen_blend;
mod tile_sampling;
mod bilinear_sampling;
mod options;
mod frame;

pub use executor::ProjectionExecutor;
pub use frame::Frame;
pub use geometry::{
    ArcPart, CartesianCamera, Polar, ProjectedArc, ScreenPoint,
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_stereographic_horizontal, project_stereographic_north, project_to_cell, rotate_camera_into, zoom_view,
    project_constellation_segment, project_horizon_labels, project_horizon_line,
};

/// Step 5: return a square image of projected stars, with no frame offsets or terminal margins.
/// The returned image borrows executor-owned storage until the next projection call.
/// Pixels use row-major order and retain floating-point RGB and opacity until final presentation. Every
/// pixel inside a tile's image is pulled back to the tile and sampled there (`pixel_warp`): a region's merged
/// tile, or the child tiles of the cells the selection split it into.
pub fn project_stars_to_screen<'a>(
    drawing_area_width_and_height: usize,
    star_regions: &[crate::model::Region],
    selected: &crate::selection::Selected,
    observation: &crate::observation::Observation,
    view: &crate::model::View,
    executor: &'a mut ProjectionExecutor,
    times: &mut StepTimes
) -> &'a [crate::model::Texel] {
    // Render in square-local coordinates; the caller places the result into the full frame.
    executor.project(drawing_area_width_and_height, star_regions, selected, observation, view, times)
}

#[cfg(test)]
mod filter_integral;
#[cfg(test)]
mod filter_summary_tests;
