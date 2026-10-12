//! Verify raw inputs only when conversion consumes them. Repair one block under its captured remote identity.
use super::{Job, transfer};
use crate::{catalog::mast::{pipeline::{MastBlock, Footer, validate_raw_block}, storage::{BlobStore, decode, invalid},
    parquet, transport::{Fetch, Http, TransferControl}}, progress::DatasetProgress, timing::StepTimes};
use std::{io, time::{Instant, Duration}, sync::mpsc, thread, panic::{catch_unwind, AssertUnwindSafe, resume_unwind}};

pub(crate) fn read_or_repair_block(store: &BlobStore, block: &mut MastBlock, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Vec<u8>> {
    if let Some(bytes) = read_cached_block(store, block, progress, times)? { return Ok(bytes); }
    repair_with_fetch(store, block, &Http::new(), progress, times) // create a client only when a repair actually needs the network
}

#[cfg(test)]
pub(in crate::catalog::mast) fn read_or_repair_with_fetch(store: &BlobStore, block: &mut MastBlock, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Vec<u8>> {
    if let Some(bytes) = read_cached_block(store, block, progress, times)? { return Ok(bytes); }
    repair_with_fetch(store, block, fetch, progress, times)
}

fn read_cached_block(store: &BlobStore, block: &MastBlock, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Option<Vec<u8>>> {
    match times.measure("MAST source block read", || read_validated_block(store, block, progress)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof) => {
            writeln!(progress, "Repairing cached MAST block {}: {error}; refetching its pinned source.", block.key)?;
            Ok(None)
        },
        Err(error) => Err(error),                                // permissions and unrelated I/O failures are not corrupt-data repairs
    }
}

fn repair_with_fetch(store: &BlobStore, block: &mut MastBlock, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Vec<u8>> {
    progress.set_detail("Repairing cached source block from its pinned source");
    let result = times.measure_steps("MAST source block repair", |times| repair_block(store, block, fetch, progress, times));
    progress.set_detail("Continuing block preparation");        // preserve the parent bar, completed count and elapsed time
    result.map_err(|error| io::Error::new(error.kind(), format!("MAST block {} repair failed: {error}", block.key)))
}

fn read_validated_block(store: &BlobStore, block: &MastBlock, progress: &mut dyn DatasetProgress) -> io::Result<Vec<u8>> {
    let bytes = store.read_object(&block.hash, progress)?;
    validate_raw_block(&bytes, block.rows).map_err(|error| invalid(format!("invalid cached Parquet block: {error}")))?;
    Ok(bytes)
}

fn repair_block(store: &BlobStore, block: &mut MastBlock, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Vec<u8>> {
    let source = block.source.as_ref().ok_or_else(|| invalid("missing pinned block provenance"))?;
    let footer: Footer = decode(&store.read_object(&source.footer_hash, &mut crate::progress::NoProgress(std::io::sink()))?)?;
    let metadata = parquet::parse_footer(&footer.bytes)?;
    if source.group >= metadata.num_row_groups() || metadata.row_group(source.group).num_rows() != block.rows as i64 {
        return Err(invalid("pinned row-group shape mismatch"));
    }
    let ranges = parquet::selected_ranges(&metadata, source.group, footer.identity.size)?;
    let compressed = ranges.iter().try_fold(0usize, |total, (_, n)| total.checked_add(*n)).ok_or_else(|| invalid("range size overflow"))?;
    let job = Job { index: 0, path: source.path.clone(), footer_hash: source.footer_hash.clone(), group: source.group,
        key: block.key.clone(), rows: block.rows, selected: block.selected, base: block.base, ranges, compressed };
    let input = times.measure("MAST repair transfer", || download_repair(store, &job, fetch, progress))?;
    progress.set_detail("Checking and saving repaired source block");
    let bytes = times.measure("MAST repair normalization", || transfer::normalize_block(input, &job))?;
    validate_raw_block(&bytes, block.rows)?;
    let published = times.measure("MAST repair publication", || store.publish(&block.key, &bytes, false))?;
    block.hash = published.hash;                                // only reconstruction from the pinned source can replace our captured identity
    block.stamp = published.stamp;
    Ok(published.bytes)
}

/// Poll the existing byte counter while the repair runs; only the calling thread touches the loading bar.
fn download_repair(store: &BlobStore, job: &Job, fetch: &impl Fetch, progress: &mut dyn DatasetProgress) -> io::Result<parquet::Ranges> {
    let control = TransferControl::default();
    let started = Instant::now();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(1);
        let control = &control;
        let worker = thread::Builder::new().name("mast-repair".into()).spawn_scoped(scope, move || {
            let _ = sender.send(transfer::download_ranges(store, job, fetch, control, started));
        })?;
        let result = catch_unwind(AssertUnwindSafe(|| loop {
            progress.set_download_bytes(control.received_bytes(), started.elapsed());
            match receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(result) => {
                    progress.set_download_bytes(control.received_bytes(), started.elapsed());
                    break result;
                },
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(io::Error::other("repair worker disconnected")),
            }
        }));
        control.stop();                                                                       // wake retries before joining, including when the progress reporter panics
        drop(receiver);
        let joined = worker.join();
        match result {
            Err(panic) => resume_unwind(panic),
            Ok(result) => {
                joined.map_err(|_| io::Error::other("repair worker panicked"))?;
                result
            },
        }
    })
}
