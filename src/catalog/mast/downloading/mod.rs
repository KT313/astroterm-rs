//! Plan a physical prefix, overlap bounded downloads, and publish blocks in the coordinator.
mod scheduling;
mod metadata;
#[cfg(test)]
pub(super) use scheduling::run_with_budget;
mod transfer;
mod repair;
pub(crate) use repair::read_or_repair_block;
#[cfg(test)]
pub(super) use repair::read_or_repair_with_fetch;

use super::{pipeline::{Leaf, Footer, MastBlock, MastSelection, MastSource},
    storage::{BlobStore, BlobStamp, VerifiedBlob, digest, encode, invalid}, transport::Fetch, parquet};
use crate::{progress::{DatasetProgress, Stage, Unit, Outcome}, time::StepTimes};
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
    progress.begin_stage(Stage::PlanDownloads, "MAST", Unit::Work, None);
    times.measure("Dataset loading.Dataset resolution.MAST metadata planning");
    let result = metadata::plan_in_order(selection, leaves, fetch, progress, times);
    progress.finish_stage(if result.is_ok() { Outcome::Complete } else { Outcome::Failed });
    result
}

fn append_file(selection: &mut MastSelection, plan: &mut Plan, path: &str, footer: Footer, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    let footer_hash = digest(&encode(&footer)?);                                             // refer to the immutable footer instead of retaining all footer bytes
    let metadata = parquet::parse_footer(&footer.bytes)?;
    for group in 0..metadata.num_row_groups() {
        if selection.available >= u64::from(selection.requested) { break; }
        let rows = usize::try_from(metadata.row_group(group).num_rows()).map_err(io::Error::other)?;
        if rows == 0 { continue; }
        let ranges = parquet::selected_ranges(&metadata, group, footer.identity.size)?;
        let compressed = ranges.iter().try_fold(0usize, |sum, (_, n)| sum.checked_add(*n)).ok_or_else(|| invalid("MAST byte count overflow"))?;
        let job = Job {
            index: plan.ready.len(), path: path.to_owned(), footer_hash: footer_hash.clone(), group,
            key: format!("raw-v1:{}:{path}:{}:{}:{group}", selection.discovery, footer.identity.etag, footer.identity.size),
            rows, selected: rows.min((u64::from(selection.requested) - selection.available) as usize), base: selection.available, ranges, compressed,
        };
        times.measure("Dataset loading.Dataset resolution.MAST metadata planning.MAST cached block lookup");
        let cached = selection.store.locate(&job.key);
        let ready = match cached {
            Ok(Some(blob)) => {
                progress.set_detail(&format!("Located {} cached bytes; payload not read", blob.bytes));
                Some(describe_block(&job, blob.hash, blob.stamp))
            },
            Ok(None) => None,
            Err(error) => {
                writeln!(progress, "Ignoring invalid cached MAST location: {error}; refetching the original input.")?;
                None
            },
        };
        if ready.is_none() { plan.jobs.push(job); }
        plan.ready.push(ready);
        selection.available = selection.available.checked_add(rows as u64).ok_or_else(|| invalid("MAST row count overflow"))?;
    }
    Ok(())
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
    describe_block(job, blob.hash, blob.stamp)
}
fn describe_block(job: &Job, hash: String, stamp: BlobStamp) -> MastBlock {
    MastBlock { key: job.key.clone(), hash, rows: job.rows, selected: job.selected, base: job.base, stamp,
        source: Some(MastSource { path: job.path.clone(), footer_hash: job.footer_hash.clone(), group: job.group }) }
}
