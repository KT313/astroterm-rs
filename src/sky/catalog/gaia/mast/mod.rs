//! Reuse qualified per-block records, then build the selected catalog with existing sorting and indexing.
mod conversion;
mod assembly;

use crate::{catalog::{datasets::DatasetDirectories, resolve_mast, mast_digest, encode_mast, BlobStore, MastSelection}, model::PreparedCatalog,
    progress::DatasetProgress, timing::StepTimes};
use std::{io, path::{Path, PathBuf}};

pub(crate) fn load_mast_catalog(count: u32, parallel_blocks: u8, directories: &DatasetDirectories, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    if !(1..=crate::constants::MAST_MAX_PARALLEL_BLOCKS).contains(&parallel_blocks) { return Err(io::Error::other(format!("MAST parallel block count must be between 1 and {}", crate::constants::MAST_MAX_PARALLEL_BLOCKS))); }
    writeln!(progress, "Gaia DR3: using G magnitude in the existing V-magnitude field; no G-to-V conversion. The star threshold uses G.")?;
    if count == 0 { return super::prepare_gaia_catalog(&[], progress, times); }                     // zero needs neither storage nor network
    let cache_root = directories.cache.as_ref().ok_or_else(|| io::Error::other("Cannot find the cache directory for reusable MAST conversions"))?;
    let mut selected = times.measure_steps("Dataset resolution", |times| resolve_mast(count, parallel_blocks, directories, progress, times))?;
    prepare_selected_catalog(&mut selected, cache_root, progress, times)
}

fn prepare_selected_catalog(selected: &mut MastSelection, cache_root: &Path, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let converted = BlobStore::new(cache_root.join("gaia-gdr3/mast-converted-v1"));

    // identify the whole selection: pinned discovery, requested and effective counts, policy and every selected block
    let policy = conversion::fingerprint();
    let (path, fingerprint) = selection_cache_identity(selected, &policy, cache_root)?;

    // reuse the prepared whole selection when its inputs are untouched, otherwise assemble from converted blocks
    if let Some(prepared) = super::super::cache::try_load_mast_prepared(&path, &fingerprint, progress, times)? {
        selected.check_unchanged()?;
        return Ok(prepared);
    }
    let prepared = times.measure_steps("Gaia loading", |times| assembly::assemble_selected_catalog(selected, &converted, &policy, progress, times))?;
    selected.check_unchanged()?;                                                                   // check for later source changes without reading every raw blob again
    let (path, fingerprint) = selection_cache_identity(selected, &policy, cache_root)?; // use final identities after any local repair
    save_prepared_catalog(path, &prepared, &fingerprint, progress, times)?;
    Ok(prepared)
}

fn selection_cache_identity(selected: &MastSelection, policy: &str, cache_root: &Path) -> io::Result<(Option<PathBuf>, [u8; 32])> {
    let identities: Vec<_> = selected.blocks.iter().map(|block| (&block.key, &block.hash, block.selected)).collect();
    let effective = selected.available.min(u64::from(selected.requested));
    let hash = mast_digest(&encode_mast(&(1u8, &selected.discovery, selected.requested, effective, &policy, &identities))?);
    let fingerprint: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(hash.as_bytes()).into();
    let path = Some(cache_root.join(format!("gaia-gdr3-mast-{hash}.catalog")));
    Ok((path, fingerprint))
}

fn save_prepared_catalog(path: Option<std::path::PathBuf>, prepared: &PreparedCatalog, fingerprint: &[u8; 32], progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    if let Some(path) = path && let Err(error) = crate::sky::write_cached_catalog(&path, prepared, fingerprint, &mut crate::progress::Report::live(progress, times)) {
        progress.finish_stage(crate::progress::Outcome::Failed);
        writeln!(progress, "Could not save prepared MAST catalog: {error}; converted blocks remain available.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{Stage, Unit, Outcome};
    #[derive(Default)]
    struct Reporter { active: Option<Stage>, outcomes: Vec<(Stage, Outcome)>, text: Vec<u8> }
    impl io::Write for Reporter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.text.extend_from_slice(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Reporter {
        fn begin_stage(&mut self, stage: Stage, _: &str, _: Unit, _: Option<u64>) { self.active = Some(stage); }
        fn finish_stage(&mut self, outcome: Outcome) {
            if let Some(stage) = self.active.take() { self.outcomes.push((stage, outcome)); }
        }
    }
    #[test]
    fn optional_write_failure_reports_failed_finalization_before_returning_catalog() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("occupied");
        std::fs::create_dir(&path).unwrap();
        let catalog = crate::sky::prepare_owned_catalog(crate::catalog::Catalog::new(Vec::new(), crate::catalog::StarNames::default(), Vec::new()), &mut crate::progress::Report::quiet()).unwrap();
        let mut reporter = Reporter::default();
        save_prepared_catalog(Some(path.clone()), &catalog, &[0; 32], &mut reporter, &mut StepTimes::default()).unwrap();
        assert!(path.is_dir());
        assert_eq!(reporter.outcomes.last(), Some(&(Stage::FinalizeCache, Outcome::Failed)));
        assert!(!reporter.outcomes.contains(&(Stage::FinalizeCache, Outcome::Complete)));
        assert!(String::from_utf8(reporter.text).unwrap().contains("Could not save prepared MAST catalog"));
    }
}
