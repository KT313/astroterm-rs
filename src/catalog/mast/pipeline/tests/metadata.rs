use super::*;
use crate::catalog::mast::transport::{Response, TransferControl};
use std::sync::{Condvar, atomic::AtomicUsize};

#[derive(Default)]
struct Gate { started: usize, finished: Vec<usize> }
struct MetadataServer {
    source: Server,
    gate: (Mutex<Gate>, Condvar),
    active: AtomicUsize,
    peak: AtomicUsize,
    fail: Option<usize>,
    panic: Option<usize>,
}
impl MetadataServer {
    fn new(leaves: usize) -> Self {
        let mut source = Server::new();
        source.leaves = leaves;
        Self { source, gate: (Mutex::new(Gate::default()), Condvar::new()), active: 0.into(), peak: 0.into(), fail: None, panic: None }
    }
}
struct Active<'a> { server: &'a MetadataServer, file: usize }
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.server.active.fetch_sub(1, Ordering::SeqCst);
        self.server.gate.0.lock().unwrap().finished.push(self.file);
        self.server.gate.1.notify_all();
    }
}
impl Fetch for MetadataServer {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
        if range.is_none() || identity.is_some() { return self.source.fetch(path, range, identity, limit, deadline); }
        let file = path.split("Npix=").nth(1).unwrap().trim_end_matches(".parquet").parse().unwrap();
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = Active { server: self, file };
        self.peak.fetch_max(active, Ordering::SeqCst);
        let mut state = self.gate.0.lock().unwrap();
        state.started += 1;
        self.gate.1.notify_all();
        let expected = self.source.leaves.min(crate::constants::MAST_METADATA_REQUESTS);
        let (state, _) = self.gate.1.wait_timeout_while(state, Duration::from_secs(5), |state| {
            state.started < expected || (file == 0 && state.finished.len() < expected - 1)
        }).unwrap();
        if state.started < expected { return Err(io::Error::other("metadata requests did not overlap")); }
        drop(state);
        if self.panic == Some(file) { panic!("injected metadata panic"); }
        if self.fail == Some(file) { return Err(io::Error::other("injected metadata failure")); }
        self.source.fetch(path, range, identity, limit, deadline)
    }
}

#[test]
fn metadata_overlaps_with_bounded_lookahead_but_selection_stays_in_file_order() {
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let limit = crate::constants::MAST_METADATA_REQUESTS;
    let server = MetadataServer::new(limit + 4);
    let requested = (limit as u32 + 2) * 3;
    let mut progress = BlockProgress::default();
    let selection = resolve_selection(requested, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
    assert_eq!(server.peak.load(Ordering::SeqCst), crate::constants::MAST_METADATA_REQUESTS);
    assert_ne!(server.gate.0.lock().unwrap().finished[0], 0);                                    // later HTTP responses deliberately finish before the first one
    assert_eq!(selection.blocks.len(), limit + 2);
    assert!(progress.details.iter().any(|detail| detail.starts_with("Waiting for file metadata:")));
    assert!(progress.details.iter().all(|detail| !detail.contains("requests in parallel")));
    for (index, block) in selection.blocks.iter().enumerate() {
        assert_eq!((block.base, block.selected), (index as u64 * 3, 3));
        assert!(block.source.as_ref().unwrap().path.ends_with(&format!("Npix={index}.parquet")));
    }
    let calls = server.source.calls.lock().unwrap().len();
    server.source.offline.store(true, Ordering::Relaxed);
    resolve_selection(requested, 2, store, &server.source, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap();
    assert_eq!(server.source.calls.lock().unwrap().len(), calls);                                // a cached short prefix never requests later metadata
}

#[test]
fn unused_metadata_errors_and_panics_do_not_reject_a_short_prefix() {
    for panic in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut server = MetadataServer::new(4);
        if panic { server.panic = Some(1); } else { server.fail = Some(1); }
        let mut progress = BlockProgress::default();
        let selected = resolve_selection(1, 2, BlobStore::new(root.path().into()), &server, &mut progress, &mut StepTimes::default()).unwrap();
        assert_eq!((selected.blocks.len(), selected.blocks[0].selected), (1, 1));
        assert!(progress.details.iter().any(|detail| detail == "Finishing in-flight metadata requests"));
        assert_eq!(server.active.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn required_metadata_failure_preserves_context_and_joins_workers() {
    for panic in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut server = MetadataServer::new(4);
        if panic { server.panic = Some(0); } else { server.fail = Some(0); }
        let error = resolve_selection(6, 2, BlobStore::new(root.path().into()), &server, &mut NoProgress(Vec::new()), &mut StepTimes::default()).err().unwrap();
        assert!(error.to_string().contains("Npix=0.parquet"), "{error}");
        assert!(error.to_string().contains(if panic { "panicked" } else { "injected metadata failure" }));
        assert_eq!(server.active.load(Ordering::SeqCst), 0);
        assert!(server.source.calls.lock().unwrap().iter().all(|call| !call.contains("bytes=") || call.contains("bytes=-"))); // star-column downloads never begin on a planning failure
    }
}

#[test]
fn progress_panic_cancels_metadata_retry_waits_and_joins_every_worker() {
    struct Waiting { source: Server, active: AtomicUsize }
    struct ActiveWait<'a>(&'a AtomicUsize);
    impl Drop for ActiveWait<'_> { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
    impl Fetch for Waiting {
        fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
            self.source.fetch(path, range, identity, limit, deadline)
        }
        fn fetch_cancellable(&self, _: &str, _: Option<&str>, _: Option<&Identity>, _: usize, _: Instant, control: &TransferControl) -> io::Result<Response> {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveWait(&self.active);
            control.pause(Duration::from_secs(5), Instant::now() + Duration::from_secs(6))?;
            Err(io::Error::other("metadata retry was not cancelled"))
        }
    }
    struct Progress<'a>(&'a AtomicUsize);
    impl std::io::Write for Progress<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Progress<'_> {
        fn set_detail(&mut self, detail: &str) {
            if !detail.starts_with("Waiting for file metadata:") { return; }
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.0.load(Ordering::SeqCst) == 0 && Instant::now() < deadline { std::thread::yield_now(); }
            assert!(self.0.load(Ordering::SeqCst) > 0);
            panic!("metadata progress panic");
        }
    }
    let root = tempfile::tempdir().unwrap();
    let server = Waiting { source: Server::new(), active: 0.into() };
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resolve_selection(4, 2, BlobStore::new(root.path().into()), &server, &mut Progress(&server.active), &mut StepTimes::default())
    })).err().unwrap();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"metadata progress panic"));
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
}
