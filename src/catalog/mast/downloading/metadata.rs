//! Bounded metadata lookahead. Only the coordinator consumes files, notices and timings in catalog order.
use super::{Plan, append_file};
use crate::{catalog::mast::{pipeline::{Leaf, Footer, MastSelection, read_cached_footer, load_footer, fetch_footer},
    storage::BlobStore, transport::{Fetch, Identity, Response, TransferControl}}, constants::MAST_METADATA_REQUESTS,
    progress::{DatasetProgress, NoProgress}, time::StepTimes};
use std::{collections::VecDeque, io, thread, time::Instant, panic::{catch_unwind, AssertUnwindSafe, resume_unwind}};

type Reply = (io::Result<Footer>, Vec<u8>);

pub(super) fn plan_in_order(selection: &mut MastSelection, leaves: Vec<Leaf>, fetch: &impl Fetch, progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> io::Result<Plan> {
    let store = selection.store.clone();
    let discovery = selection.discovery.clone();
    let control = TransferControl::default();
    thread::scope(|scope| {
        let mut pending: VecDeque<(String, thread::ScopedJoinHandle<'_, Reply>)> = VecDeque::new();
        let mut next = 0;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut plan = Plan { jobs: Vec::new(), ready: Vec::new() };
            while selection.available < u64::from(selection.requested) && (next < leaves.len() || !pending.is_empty()) {
                let required_missing = pending.is_empty();
                if required_missing {
                    let path = leaves[next].path();
                    times.measure("Dataset loading.Dataset resolution.MAST metadata planning.MAST cached file metadata");
                    if let Some(footer) = read_cached_footer(&store, &discovery, &path, progress)? {
                        progress.set_detail(&format!("Using cached file metadata: {path}"));
                        append_file(selection, &mut plan, &path, footer, progress, times)?;
                        next += 1;
                        continue;                                                             // a fully cached prefix never starts speculative network requests
                    }
                }
                while next < leaves.len() && pending.len() < MAST_METADATA_REQUESTS.max(1) {
                    let path = leaves[next].path();
                    let worker_path = path.clone();
                    let (store, discovery, control) = (&store, &discovery, &control);
                    let fetch_only = required_missing && pending.is_empty();                   // the coordinator already checked this first missing footer
                    let worker = thread::Builder::new().name("mast-metadata".into()).spawn_scoped(scope, move || {
                        load_metadata(store, discovery, &worker_path, fetch, control, fetch_only)
                    })?;
                    pending.push_back((path, worker));
                    next += 1;
                }
                let path = &pending.front().expect("a missing footer starts at least one worker").0;
                progress.set_detail(&format!("Waiting for file metadata: {path}"));
                let (path, worker) = pending.pop_front().unwrap();                              // retain ownership of every handle until progress callbacks have returned
                times.measure("Dataset loading.Dataset resolution.MAST metadata planning.MAST metadata wait");
                let (footer, notices) = worker.join()
                    .map_err(|_| io::Error::other(format!("metadata worker panicked for {path}")))?;
                progress.write_all(&notices)?;
                let footer = footer.map_err(|error| io::Error::new(error.kind(), format!("MAST metadata {path}: {error}")))?;
                times.describe("Dataset loading.Dataset resolution.MAST metadata planning.MAST metadata wait", || format!("file={path}; caller wait only; concurrent request durations are not added together"));
                append_file(selection, &mut plan, &path, footer, progress, times)?;
            }
            if !pending.is_empty() { progress.set_detail("Finishing in-flight metadata requests"); }
            Ok(plan)
        }));
        control.stop();                                                                       // cancel retry waits once the prefix is complete or a required operation fails
        for (_, worker) in pending { let _ = worker.join(); }                                  // unused metadata failures do not reject a valid shorter prefix
        match result { Ok(result) => result, Err(panic) => resume_unwind(panic) }
    })
}

fn load_metadata(store: &BlobStore, discovery: &str, path: &str, fetch: &impl Fetch, control: &TransferControl, fetch_only: bool) -> Reply {
    let mut progress = NoProgress(Vec::new());
    let fetch = MetadataFetch { inner: fetch, control };
    let result = control.check().and_then(|()| {
        if fetch_only { fetch_footer(store, discovery, path, &fetch, &mut progress) }
        else { load_footer(store, discovery, path, &fetch, &mut progress) }
    });
    (result, progress.0)
}

/// Keep the existing range/ETag checks and retry deadlines, sharing cancellation across metadata workers.
struct MetadataFetch<'a, F> { inner: &'a F, control: &'a TransferControl }
impl<F: Fetch> Fetch for MetadataFetch<'_, F> {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
        self.inner.fetch_cancellable(path, range, identity, limit, deadline, self.control)
    }
}
