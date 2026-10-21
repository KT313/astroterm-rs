//! Independent projection optimizations, chosen once when the executor is created.
use crate::constants::PROJECTION_USE_BILINEAR_FAST_PATH;

pub(super) struct ProjectionOptions {
    pub bilinear_fast_path: bool,
}

impl Default for ProjectionOptions {
    fn default() -> Self {
        Self { bilinear_fast_path: PROJECTION_USE_BILINEAR_FAST_PATH }
    }
}
