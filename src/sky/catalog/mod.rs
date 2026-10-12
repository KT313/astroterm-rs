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
pub use cache::{catalog_fingerprint, cache_path, load_sky_catalog, write_cached_catalog, load_cached_catalog};
