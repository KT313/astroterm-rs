//! Reuse qualified per-block records, then build the selected catalog with existing sorting and indexing.
mod conversion;
mod assembly;

use crate::{catalog::{datasets::DatasetDirectories, resolve_mast, mast_digest, encode_mast, BlobStore}, model::PreparedCatalog,
    progress::DatasetProgress, timing::StepTimes};
use std::io;

pub(crate) fn load_mast_catalog(count: u32, parallel_blocks: u8, directories: &DatasetDirectories, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    if !(1..=crate::constants::MAST_MAX_PARALLEL_BLOCKS).contains(&parallel_blocks) { return Err(io::Error::other(format!("MAST parallel block count must be between 1 and {}", crate::constants::MAST_MAX_PARALLEL_BLOCKS))); }
    writeln!(progress, "Gaia DR3: using G magnitude in the existing V-magnitude field; no G-to-V conversion. The star threshold uses G.")?;
    if count == 0 { return super::prepare_gaia_catalog(&[], progress, times); }                     // zero needs neither storage nor network
    let cache_root = directories.cache.as_ref().ok_or_else(|| io::Error::other("Cannot find the cache directory for reusable MAST conversions"))?;
    let selected = times.measure_steps("Dataset resolution", |times| resolve_mast(count, parallel_blocks, directories, progress, times))?;
    let converted = BlobStore::new(cache_root.join("gaia-gdr3/mast-converted-v1"));

    // identify the whole selection: pinned discovery, requested and effective counts, policy and every selected block
    let policy = conversion::fingerprint();
    let keys: Vec<_> = selected.blocks.iter().map(|block| conversion::block_key(block, &policy)).collect();
    let identities: Vec<_> = selected.blocks.iter().map(|block| (&block.key, &block.hash, block.selected)).collect();
    let effective = selected.available.min(u64::from(count));
    let hash = mast_digest(&encode_mast(&(1u8, &selected.discovery, selected.requested, effective, &policy, &identities))?);
    let fingerprint: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(hash.as_bytes()).into();
    let path = Some(cache_root.join(format!("gaia-gdr3-mast-{hash}.catalog")));

    // reuse the prepared whole selection when its inputs are untouched, otherwise assemble from converted blocks
    if let Some(prepared) = super::super::cache::try_load_mast_prepared(&path, &fingerprint, progress, times)? {
        selected.check_unchanged()?;
        return Ok(prepared);
    }
    let prepared = times.measure_steps("Gaia loading", |times| assembly::assemble_selected_catalog(&selected, &converted, &keys, progress, times))?;
    selected.check_unchanged()?;                                                                   // check for later source changes without reading every raw blob again
    if let Some(path) = path && let Err(error) = super::super::cache::write_cached_catalog(&path, &prepared, &fingerprint) {
        writeln!(progress, "Could not save prepared MAST catalog: {error}; converted blocks remain available.")?;
    }
    Ok(prepared)
}
