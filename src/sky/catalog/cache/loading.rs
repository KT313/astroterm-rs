//! Loading details keep I/O failures and optional diagnostics outside the catalog pipeline.
use crate::constants::CELL_COUNT;
use crate::model::PreparedCatalog;
use crate::catalog::{datasets::DatasetDirectories, cache::supported};
use crate::timing::StepTimes;
use sha2::{Digest, Sha256};
use std::{io, path::{Path, PathBuf}};
use super::format::{cache_path, catalog_fingerprint};

pub(super) fn load_embedded_sky(times: &mut StepTimes, notices: &mut dyn crate::progress::DatasetProgress) -> io::Result<PreparedCatalog> {
    let parsed = times
        .measure("Embedded BSC loading", || crate::catalog::load_embedded_catalog_with_progress(notices))
        .map_err(io::Error::other)?;
    times.describe("Embedded BSC loading", || {
        format!(
            "input entries={}; placeholders={}; parsed entries={} (placeholders removed during preparation)",
            parsed.stars.len(),
            parsed.stars.iter().filter(|s| !s.has_data).count(),
            parsed.stars.len()
        )
    });
    prepare_catalog(parsed, times, notices)
}

pub(super) fn resolve_cache_path(source: &Path, directories: &DatasetDirectories, notices: &mut dyn crate::progress::DatasetProgress) -> Option<PathBuf> {
    if supported() {
        directories
            .cache
            .as_ref()
            .and_then(|dir| match cache_path(source, dir) {
                Ok(path) => Some(path),
                Err(error) => {
                    let _ = writeln!(notices, "Cache unavailable: {error}");
                    None
                }
            })
    } else {
        None
    }
}

pub(super) fn fingerprint_source(path: &Option<PathBuf>) -> [u8; 32] {
    let mut fingerprint = catalog_fingerprint();
    if let Some(path) = path {
        let mut hash = Sha256::new();
        hash.update(fingerprint);
        hash.update(path.file_name().expect("cache filename").as_encoded_bytes());
        fingerprint = hash.finalize().into();
    }
    fingerprint
}

pub(crate) fn try_load_prepared_catalog(path: &Option<PathBuf>, fingerprint: &[u8; 32], notices: &mut dyn crate::progress::DatasetProgress, times: &mut StepTimes) -> io::Result<Option<PreparedCatalog>> {
    if let Some(path) = path {
        let cached = times.measure("Prepared catalog lookup", || super::format::load_cached_catalog_with_progress(path, fingerprint, notices));
        times.describe("Prepared catalog lookup", || match &cached {
            Ok(catalog) => format!("hit: validated owned stars={}; source CSV not read; original skipped-row counts unavailable in this cache format", catalog.catalog.stars.len()),
            Err(error) => format!("miss: {error}; parse source next"),
        });
        match cached {
            Ok(catalog) => {
                notices.finish_stage(crate::progress::Outcome::Complete);
                writeln!(notices, "Loaded prepared catalog: {} stars", catalog.catalog.stars.len())?;
                return Ok(Some(catalog));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Err(error),
            Err(error) => {
                notices.finish_stage(crate::progress::Outcome::Failed);
                writeln!(
                    notices,
                    "Ignoring cache {}: {error}; rebuilding from CSV.",
                    path.display()
                )?;
            }
        }
    }
    Ok(None)
}

pub(super) fn write_prepared_catalog_if_stable(path: Option<PathBuf>, source: &Path, catalog: &PreparedCatalog, fingerprint: &[u8; 32], notices: &mut dyn crate::progress::DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    if let Some(path) = path {
        if cache_path(source, path.parent().unwrap()).ok().as_ref() != Some(&path) {
            writeln!(notices, "Dataset changed while loading; cache was not written.")?;
        } else if let Err(error) = times.measure("Prepared catalog write", || {
            super::format::write_cached_catalog_with_progress(&path, catalog, fingerprint, notices)
        }) {
            notices.finish_stage(crate::progress::Outcome::Failed);
            writeln!(
                notices,
                "Could not write cache {}: {error}; continuing without a cache.",
                path.display()
            )?;
        }
    }
    Ok(())
}

pub(super) fn prepare_catalog(parsed: crate::catalog::Catalog, times: &mut crate::timing::StepTimes, notices: &mut dyn crate::progress::DatasetProgress) -> io::Result<PreparedCatalog> {
    let input = parsed.stars.len();
    let catalog = times.measure("Catalog preparation", || super::super::pipeline::prepare_owned_catalog_with_progress(parsed, notices))?;
    times.describe("Catalog preparation", || format!("input entries={input}; removed placeholders={}; output stars={}; grid cells={CELL_COUNT}; unique constellation endpoints={}", input - catalog.catalog.stars.len(), catalog.catalog.stars.len(), catalog.catalog.endpoint_indices().len()));
    Ok(catalog)
}

