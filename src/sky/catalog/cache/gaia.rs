//! Gaia's prepared cache identifies the entire selected prefix, never just its first input file.
use crate::catalog::datasets::{resolve_gaia_files, DatasetDirectories, GaiaFile};
use crate::model::PreparedCatalog;
use crate::timing::StepTimes;
use sha2::{Digest, Sha256};
use std::{io, path::PathBuf};
use super::{format::catalog_fingerprint, loading::try_load_prepared_catalog};

pub(super) fn load_gaia_catalog(count: u16, directories: &DatasetDirectories, notices: &mut dyn crate::progress::DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    writeln!(notices, "Gaia DR3: using G magnitude in the existing V-magnitude field; no G-to-V conversion. The star threshold uses G.")?;
    let files = times.measure_steps("Dataset resolution", |times| resolve_gaia_files(count, directories, notices, times))?;
    let fingerprint = fingerprint_files(&files);
    let path = directories.cache.as_ref().filter(|_| crate::catalog::cache::supported()).map(|directory| {
        directory.join(format!("gaia-gdr3-{}.catalog", fingerprint.iter().map(|b| format!("{b:02x}")).collect::<String>()))
    });
    load_prepared_gaia(&files, path, &fingerprint, notices, times)
}

fn load_prepared_gaia(files: &[GaiaFile], path: Option<PathBuf>, fingerprint: &[u8; 32], notices: &mut dyn crate::progress::DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    if let Some(catalog) = try_load_prepared_catalog(&path, fingerprint, notices, times)? {
        check_sources(files)?;
        return Ok(catalog);
    }
    let catalog = times.measure_steps("Gaia loading", |times| super::super::gaia::prepare_gaia_catalog(files, notices, times))?;
    check_sources(files)?;
    if let Some(path) = path
        && let Err(error) = super::format::write_cached_catalog_with_times(&path, &catalog, fingerprint, notices, times) {
        notices.finish_stage(crate::progress::Outcome::Failed);
        writeln!(notices, "Could not write cache {}: {error}; continuing without a cache.", path.display())?;
    }
    Ok(catalog)
}

fn check_sources(files: &[GaiaFile]) -> io::Result<()> {
    for file in files {
        if !file.is_unchanged()? { return Err(io::Error::other(format!("Gaia source changed during loading: {}", file.path.display()))); }
    }
    Ok(())
}

/// Verified name and checksum identify each input; where the file sits and when it was written do not matter.
fn fingerprint_files(files: &[GaiaFile]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(catalog_fingerprint());
    hash.update(b"Gaia DR3 G-as-V; J2016-TCB-approx-TT to J2000; RV0; Gaia-only; import-v1");
    hash.update(crate::constants::GAIA_DISTANCE_MIN_SIGNAL.to_le_bytes());
    hash.update((files.len() as u64).to_le_bytes());
    for file in files {
        hash.update(file.entry.name.as_bytes());
        hash.update(file.entry.checksum);
    }
    for source in [include_bytes!("../../../catalog/gaia/reader.rs").as_slice(), include_bytes!("../gaia/conversion.rs").as_slice(), include_bytes!("../gaia/mod.rs").as_slice(), include_bytes!("gaia.rs").as_slice()] { hash.update(source); }
    hash.finalize().into()
}

#[cfg(test)]
mod tests;
