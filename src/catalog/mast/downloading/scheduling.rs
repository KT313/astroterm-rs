//! Each unfinished block keeps its slot and byte reservation until publication or shutdown.
use super::{Plan, finish_block, transfer::{download_ranges, normalize_block}};
use crate::{catalog::mast::{storage::{BlobStore, invalid}, transport::{Fetch, TransferControl}},
    constants::{MAST_PENDING_COMPRESSED_MAX_BYTES, MAST_BLOCK_MAX_BYTES}, progress::DatasetProgress, time::StepTimes};
use std::{io, collections::HashMap, panic::{catch_unwind, AssertUnwindSafe, resume_unwind}, sync::mpsc, thread, time::{Instant, Duration}};

pub(super) fn run_downloads(store: &BlobStore, plan: &mut Plan, parallel: u8, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    run_with_budget(store, plan, usize::from(parallel), MAST_PENDING_COMPRESSED_MAX_BYTES, fetch, progress, times)
}

fn fits_budget(reserved: usize, next: usize, budget: usize) -> bool { next <= budget.saturating_sub(reserved) }

#[allow(clippy::too_many_arguments)]
pub(in crate::catalog::mast) fn run_with_budget(store: &BlobStore, plan: &mut Plan, parallel: usize, budget: usize, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<()> {
    if parallel == 0 || plan.jobs.iter().any(|job| job.compressed > budget || job.compressed > MAST_BLOCK_MAX_BYTES) {
        return Err(invalid("MAST block cannot fit the download limits"));
    }
    let control = TransferControl::default();
    let started = Instant::now();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(parallel);
        let mut handles = HashMap::new();
        let jobs = &plan.jobs;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let (mut next, mut reserved) = (0, 0);
            let mut ready = plan.ready.iter().filter(|block| block.is_some()).count();
            while next < jobs.len() || !handles.is_empty() {
                times.measure("Dataset loading.Dataset resolution.MAST block downloads.Transfer wait");
                while next < jobs.len() && handles.len() < parallel && fits_budget(reserved, jobs[next].compressed, budget) {
                    let number = next;
                    let job = &jobs[number];
                    let sender = sender.clone();
                    let control = &control;
                    let admitted = Instant::now();                                            // waiting for a slot does not consume the transfer deadline
                    let handle = thread::Builder::new().name(format!("mast-block-{number}")).spawn_scoped(scope, move || {
                        let outcome = catch_unwind(AssertUnwindSafe(|| download_ranges(store, job, fetch, control, admitted)))
                            .unwrap_or_else(|_| Err(io::Error::other("block worker panicked")));
                        let _ = sender.send((number, outcome));                                // receiver closes before shutdown joins any remaining worker
                    })?;
                    handles.insert(number, handle);
                    reserved += job.compressed;
                    next += 1;
                }
                if handles.is_empty() { return Err(invalid("MAST download scheduler made no progress")); }
                progress.set_download_bytes(control.received_bytes(), started.elapsed());
                let (number, outcome) = match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(result) => result,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,                           // refresh the rate even while no block has finished
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("MAST block worker disconnected")),
                };
                handles.remove(&number).ok_or_else(|| invalid("unexpected MAST block completion"))?.join()
                    .map_err(|_| io::Error::other("MAST block worker panicked after reporting"))?;
                let job = &jobs[number];
                let finish = || {
                    let input = outcome?;
                    times.measure("Dataset loading.Dataset resolution.MAST block downloads.MAST block normalization");
                    let raw = normalize_block(input, job)?;
                    times.measure("Dataset loading.Dataset resolution.MAST block downloads.MAST block publication");
                    store.publish(&job.key, &raw, false)
                };
                let blob = finish().map_err(|error: io::Error| io::Error::new(error.kind(), format!("{} group {}: {error}", job.path, job.group)))?;
                plan.ready[job.index] = Some(finish_block(job, blob));                           // completion order never determines catalog order
                reserved -= job.compressed;
                ready += 1;
                progress.set_download_bytes(control.received_bytes(), started.elapsed());
                progress.set_position(ready as u64);                                           // count only fully installed, reusable blocks
            }
            Ok(())
        }));
        control.stop();
        drop(receiver);                                                                       // release blocked senders before waiting for workers, including on panic
        drop(sender);
        for (_, handle) in handles { let _ = handle.join(); }
        match result { Ok(result) => result, Err(panic) => resume_unwind(panic) }
    })
}

#[cfg(test)]
mod tests;
