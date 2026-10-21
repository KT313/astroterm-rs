//! Resolve the selected dataset, reuse validated prepared data, or prepare the source catalog.
use crate::catalog::datasets::{Dataset, DatasetDirectories};
use crate::model::PreparedCatalog;
use crate::time::StepTimes;
use std::io::{self, Write};
use crate::progress::{DatasetProgress, Report};
use super::loading::{
    load_embedded_sky, resolve_cache_path, fingerprint_source, try_load_prepared_catalog, prepare_catalog,
    write_prepared_catalog_if_stable,
};

/// Resolve/download before opening the terminal, reuse validated prepared data or prepare the source catalog.
/// Cache failures are notices on `notices`; the source remains authoritative. Startup timings go to `times`;
/// cache-loaded catalogs are not reparsed for statistics.
pub fn load_sky_catalog(dataset: Option<&Dataset>, parallel_blocks: u8, directories: &DatasetDirectories, report: &mut Report<'_>) -> io::Result<PreparedCatalog> {
    let (notices, times) = report.parts();
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
    times.measure("Dataset loading.Dataset resolution");
    let source = crate::catalog::datasets::resolve_dataset(dataset, directories, notices)?;
    times.describe("Dataset loading.Dataset resolution", || format!("source={}", source.display()));
    let path = resolve_cache_path(&source, directories, notices); // find an optional prepared-cache location
    let fingerprint = fingerprint_source(&path);                // identify the source and all preparation rules
    if let Some(catalog) = try_load_prepared_catalog(&path, &fingerprint, notices, times)? { return report_clipping(catalog, notices); }

    times.measure("Dataset loading.AT-HYG loading");
    let parsed = crate::catalog::load_athyg_catalog(&source, &mut Report::live(notices, times)).map_err(io::Error::other)?;
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
