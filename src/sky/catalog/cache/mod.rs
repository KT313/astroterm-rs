//! Prepared-catalog loading, source fingerprints and mapped-file validation.
mod pipeline;
mod loading;
mod format;

pub use pipeline::{load_sky_catalog, load_sky_catalog_with_times};
pub use format::{catalog_fingerprint, cache_path, write_cached_catalog, load_cached_catalog};
