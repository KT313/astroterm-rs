//! Reuse qualified per-block records, then build the selected catalog with existing sorting and indexing.
mod conversion;
mod assembly;

use crate::{catalog::{datasets::DatasetDirectories, resolve_mast, mast_digest, encode_mast, BlobStore}, model::PreparedCatalog, progress::DatasetProgress, timing::StepTimes};
use std::io;

pub(crate) fn load_mast_catalog(count: u32, directories: &DatasetDirectories, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    writeln!(progress, "Gaia DR3: using G magnitude in the existing V-magnitude field; no G-to-V conversion. The star threshold uses G.")?;
    if count == 0 { return super::prepare_gaia_catalog(&[], progress, times); }
    let cache_root = directories.cache.as_ref().ok_or_else(|| io::Error::other("Cannot find the cache directory for reusable MAST conversions"))?;
    let selected = times.measure_steps("Dataset resolution", |times| resolve_mast(count, directories, progress, times))?;
    let converted = BlobStore::new(cache_root.join("gaia-gdr3/mast-converted-v1"));
    let policy = conversion::fingerprint();
    let keys: Vec<_> = selected.blocks.iter().map(|block| conversion::block_key(block, &policy)).collect();
    let identities: Vec<_> = selected.blocks.iter().map(|b| (&b.key,&b.hash,b.selected)).collect();
    let hash = mast_digest(&encode_mast(&(1u8,&selected.discovery,selected.requested,selected.available.min(u64::from(count)),&policy,&identities))?);
    let fingerprint: [u8;32] = <sha2::Sha256 as sha2::Digest>::digest(hash.as_bytes()).into();
    let path = Some(cache_root.join(format!("gaia-gdr3-mast-{hash}.catalog")));
    if let Some(prepared) = super::super::cache::try_load_mast_prepared(&path, &fingerprint, progress, times)? { return Ok(prepared); }
    let prepared = times.measure_steps("Gaia loading", |times| assembly::assemble_selected_catalog(&selected, &converted, &keys, progress, times))?;
    for block in &selected.blocks { selected.read(block)?; } // verify raw identities before publishing combined output
    if let Some(path) = path && let Err(e) = super::super::cache::write_cached_catalog(&path,&prepared,&fingerprint) {
        writeln!(progress,"Could not save prepared MAST catalog: {e}; converted blocks remain available.")?;
    }
    Ok(prepared)
}
