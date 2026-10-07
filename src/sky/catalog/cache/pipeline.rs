//! Resolve the selected dataset, reuse validated prepared data, or prepare the source catalog.
use crate::catalog::{datasets::{Dataset, DatasetDirectories, resolve_dataset}, load_athyg_catalog_with_times};
use crate::model::PreparedCatalog;
use crate::timing::StepTimes;
use std::io::{self, Write};
use super::loading::{
    load_embedded_sky, resolve_cache_path, fingerprint_source, try_load_prepared_catalog, prepare_catalog,
    write_prepared_catalog_if_stable,
};

/// Resolve/download before opening the terminal. Cache failures are notices; the source remains authoritative.
pub fn load_sky_catalog(
    dataset: Option<&Dataset>,
    directories: &DatasetDirectories,
    notices: &mut impl Write,
) -> io::Result<PreparedCatalog> {
    load_sky_catalog_with_times(dataset, directories, notices, &mut StepTimes::default())
}

/// The production loader with optional startup diagnostics; cache-loaded catalogs are not reparsed for statistics.
pub fn load_sky_catalog_with_times(
    dataset: Option<&Dataset>,
    directories: &DatasetDirectories,
    notices: &mut impl Write,
    times: &mut StepTimes,
) -> io::Result<PreparedCatalog> {
    let Some(dataset) = dataset else { return load_embedded_sky(times); }; // use the built-in catalog unless another dataset was selected
    let source = times.measure("Dataset resolution", || resolve_dataset(dataset, directories, notices))?;
    times.describe("Dataset resolution", || format!("source={}", source.display()));
    let path = resolve_cache_path(&source, directories, notices); // find an optional prepared-cache location
    let fingerprint = fingerprint_source(&path);                // identify the source and all preparation rules
    if let Some(catalog) = try_load_prepared_catalog(&path, &fingerprint, notices, times)? { return Ok(catalog); }

    let parsed = times.measure_steps("AT-HYG loading", |times| load_athyg_catalog_with_times(&source, times)).map_err(io::Error::other)?;
    let catalog = prepare_catalog(parsed, times);               // compact and index the parsed stars
    write_prepared_catalog_if_stable(path, &source, &catalog, &fingerprint, notices, times)?; // save only if the source stayed unchanged
    Ok(catalog)
}
