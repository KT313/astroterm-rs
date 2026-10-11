//! Plan a physical prefix, overlap bounded downloads, and publish blocks in the coordinator.
mod scheduling;
#[cfg(test)]
pub(super) use scheduling::run_with_budget;
mod transfer;

use super::{pipeline::{Leaf, Footer, MastBlock, MastSelection, load_footer, recoverable_read, validate_raw_block},
    storage::{BlobStore, VerifiedBlob, digest, encode, invalid}, transport::Fetch, parquet};
use crate::{progress::{DatasetProgress, Stage, Unit, Outcome}, timing::StepTimes};
use std::io;

pub(super) struct Job {
    pub index: usize,
    pub path: String,
    pub footer_hash: String,
    pub group: usize,
    pub key: String,
    pub rows: usize,
    pub selected: usize,
    pub base: u64,
    pub ranges: Vec<(u64, usize)>,
    pub compressed: usize,
}
pub(super) struct Plan { pub jobs: Vec<Job>, pub ready: Vec<Option<MastBlock>> }

pub(super) fn plan_blocks(selection: &mut MastSelection, leaves: Vec<Leaf>, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Plan> {
    let mut plan = Plan { jobs: Vec::new(), ready: Vec::new() };
    for leaf in leaves {
        if selection.available >= u64::from(selection.requested) { break; }
        let path = leaf.path();
        let footer = times.measure("MAST file metadata", || load_footer(&selection.store, &selection.discovery, &path, fetch, progress))?;
        let footer_hash = digest(&encode(&footer)?);                                             // refer to the immutable footer instead of retaining all footer bytes
        let metadata = parquet::parse_footer(&footer.bytes)?;
        for group in 0..metadata.num_row_groups() {
            if selection.available >= u64::from(selection.requested) { break; }
            let rows = usize::try_from(metadata.row_group(group).num_rows()).map_err(io::Error::other)?;
            if rows == 0 { continue; }
            let ranges = parquet::selected_ranges(&metadata, group, footer.identity.size)?;
            let compressed = ranges.iter().try_fold(0usize, |sum, (_, n)| sum.checked_add(*n)).ok_or_else(|| invalid("MAST byte count overflow"))?;
            let job = Job {
                index: plan.ready.len(), path: path.clone(), footer_hash: footer_hash.clone(), group,
                key: format!("raw-v1:{}:{path}:{}:{}:{group}", selection.discovery, footer.identity.etag, footer.identity.size),
                rows, selected: rows.min((u64::from(selection.requested) - selection.available) as usize), base: selection.available, ranges, compressed,
            };
            let cached = recoverable_read(&selection.store, &job.key, progress)?;
            let ready = cached.map(|blob| { validate_raw_block(&blob.bytes, rows)?; Ok::<_, io::Error>(finish_block(&job, blob)) }).transpose()?;
            if ready.is_none() { plan.jobs.push(job); }
            plan.ready.push(ready);
            selection.available = selection.available.checked_add(rows as u64).ok_or_else(|| invalid("MAST row count overflow"))?;
        }
    }
    Ok(plan)
}

pub(super) fn download_missing(store: &BlobStore, plan: &mut Plan, parallel: u8, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    if plan.jobs.is_empty() { return Ok(()); }
    let ready = plan.ready.iter().filter(|block| block.is_some()).count();
    progress.begin_stage(Stage::Download, "MAST blocks ready", Unit::Blocks, Some(plan.ready.len() as u64));
    progress.set_cached_blocks(ready as u64);                                                        // existing blocks are already ready before any network work starts
    let result = scheduling::run_downloads(store, plan, parallel, fetch, progress, times);
    progress.finish_stage(if result.is_ok() { Outcome::Complete } else { Outcome::Failed });
    result
}

fn finish_block(job: &Job, blob: VerifiedBlob) -> MastBlock {
    MastBlock { key: job.key.clone(), hash: blob.hash, rows: job.rows, selected: job.selected, base: job.base, stamp: blob.stamp }
}
