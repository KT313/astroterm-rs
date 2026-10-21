//! Catalog loading and preparation: parsers' rows become the prepared catalog, and the prepared catalog
//! becomes the sky tables the frame loop reads. Every symbol is exported from here.
mod catalog;

pub use catalog::{prepare_catalog, prepare_owned_catalog, prepare_constellation_set, prepare_star, build_sky_tables};
pub use catalog::{catalog_fingerprint, cache_path, load_sky_catalog, write_cached_catalog, load_cached_catalog};
