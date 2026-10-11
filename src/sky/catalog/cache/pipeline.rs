//! Resolve the selected dataset, reuse validated prepared data, or prepare the source catalog.
use crate::catalog::datasets::{Dataset, DatasetDirectories};
use crate::model::PreparedCatalog;
use crate::timing::StepTimes;
use std::io::{self, Write};
use crate::progress::{DatasetProgress, NoProgress};
use super::loading::{
    load_embedded_sky, resolve_cache_path, fingerprint_source, try_load_prepared_catalog, prepare_catalog,
    write_prepared_catalog_if_stable,
};

/// Resolve/download before opening the terminal. Cache failures are notices; the source remains authoritative.
pub fn load_sky_catalog(
    dataset: Option<&Dataset>,
    directories: &DatasetDirectories,
    notices: &mut (impl Write + ?Sized),
) -> io::Result<PreparedCatalog> {
    load_sky_catalog_with_times(dataset, crate::constants::MAST_PARALLEL_BLOCKS, directories, &mut NoProgress(notices), &mut StepTimes::default())
}

/// The production loader with optional startup diagnostics; cache-loaded catalogs are not reparsed for statistics.
pub fn load_sky_catalog_with_times(dataset: Option<&Dataset>, parallel_blocks: u8, directories: &DatasetDirectories,
    notices: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let result = load_reported_catalog(dataset, parallel_blocks, directories, notices, times);
    notices.finish_stage(if result.is_ok() { crate::progress::Outcome::Complete } else { crate::progress::Outcome::Failed });
    result
}

fn load_reported_catalog(dataset: Option<&Dataset>, parallel_blocks: u8, directories: &DatasetDirectories,
    notices: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let Some(dataset) = dataset else { return report_clipping(load_embedded_sky(times, notices)?, notices); }; // use the built-in catalog unless another dataset was selected
    if let Dataset::GaiaGdr3Stars { stars } = dataset {
        return report_clipping(super::super::gaia::load_mast_catalog(*stars, parallel_blocks, directories, notices, times)?, notices);
    }
    if let Dataset::GaiaGdr3 { files } = dataset {
        return report_clipping(super::gaia::load_gaia_catalog(*files, directories, notices, times)?, notices);
    }
    let source = times.measure("Dataset resolution", || crate::catalog::datasets::resolve_dataset(dataset, directories, notices))?;
    times.describe("Dataset resolution", || format!("source={}", source.display()));
    let path = resolve_cache_path(&source, directories, notices); // find an optional prepared-cache location
    let fingerprint = fingerprint_source(&path);                // identify the source and all preparation rules
    if let Some(catalog) = try_load_prepared_catalog(&path, &fingerprint, notices, times)? { return report_clipping(catalog, notices); }

    let parsed = times.measure_steps("AT-HYG loading", |times| crate::catalog::load_athyg_catalog_with_times(&source, times, notices)).map_err(io::Error::other)?;
    let catalog = prepare_catalog(parsed, times, notices)?;               // compact and index the parsed stars
    write_prepared_catalog_if_stable(path, &source, &catalog, &fingerprint, notices, times)?; // save only if the source stayed unchanged
    report_clipping(catalog, notices)
}

fn report_clipping(catalog: PreparedCatalog, notices: &mut (impl Write + ?Sized)) -> io::Result<PreparedCatalog> {
    let counts = catalog.catalog.stars.magnitude_clipping();
    if counts.any() {
        writeln!(notices, "Warning: {} Lower clips: {}; upper clips: {}.", crate::catalog::MAGNITUDE_CLIPPING_WARNING, counts.lower, counts.upper)?;
    }
    Ok(catalog)
}
