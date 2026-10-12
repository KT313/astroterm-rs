//! Catalog preparation, conservative selection and prepared-file loading.
mod pipeline;
mod gaia;
mod preparation;
mod grid;
mod cache;
mod stars;
mod tables;

pub use pipeline::{prepare_catalog, prepare_owned_catalog, prepare_constellation_set};
pub use stars::prepare_star;
pub use tables::build_sky_tables;
pub use grid::select_grid;
pub(crate) use grid::{select_region, select_brightness, count_region_stars};
pub use cache::{catalog_fingerprint, cache_path, load_sky_catalog, load_sky_catalog_with_times, write_cached_catalog, load_cached_catalog};
