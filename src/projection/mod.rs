//! Public projection operations. Geometry and cache implementations stay private.
mod caching;
mod geometry;
mod pipeline;

pub use caching::{borrow_projected};
pub use pipeline::project_cached_sky;
pub use geometry::{
    find_visible_arc_parts, find_visible_arc_parts_vectors, pan_view, polar_to_cell, prepare_camera,
    project_camera, project_equidistant_horizontal, project_horizontal, project_light_direction,
    project_sky, project_sky_with_times, project_stereographic_horizontal, project_stereographic_north,
    project_to_cell, select_view_region, zoom_view,
};
#[cfg(test)]
pub(crate) use geometry::{compute_visible_horizon_half_range, project_constellation_segment, project_horizon_labels, project_horizon_line};
