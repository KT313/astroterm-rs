use super::*;
use crate::progress::NoProgress;
use ::parquet::schema::parser::parse_message_type;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use super::super::parquet::Ranges;
use bytes::Bytes;

fn row(id: u64) -> parquet::MastRow {
    parquet::MastRow { source_id: id, values: [45., 0., 2016., 1., 2., 1., 20., 0., 12., 13., 11.].map(|v| Some(f64::to_bits(v))) }
}
fn schema() -> ::parquet::schema::types::Type {
    parse_message_type("message gaia {
        OPTIONAL INT64 source_id;

        OPTIONAL DOUBLE ra; OPTIONAL DOUBLE dec; OPTIONAL DOUBLE ref_epoch;
        OPTIONAL DOUBLE pmra; OPTIONAL DOUBLE pmdec; OPTIONAL DOUBLE parallax;
        OPTIONAL FLOAT parallax_over_error; OPTIONAL FLOAT radial_velocity;
        OPTIONAL FLOAT phot_g_mean_mag; OPTIONAL FLOAT phot_bp_mean_mag; OPTIONAL FLOAT phot_rp_mean_mag;
    }").unwrap()
}
struct Server {
    calls: Mutex<Vec<String>>,
    offline: AtomicBool,
    changed: AtomicBool,
    changed_discovery: AtomicBool,
    data: Vec<u8>,
    leaves: usize,
}
impl Server {
    fn new() -> Self {
        Self { calls: Mutex::new(Vec::new()), offline: AtomicBool::new(false), changed: AtomicBool::new(false),
            changed_discovery: AtomicBool::new(false), leaves: 2, data: parquet::write_block(&[row(10), row(20), row(30)], schema()).unwrap() }
    }
}
impl Fetch for Server {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, _: Instant) -> io::Result<super::super::transport::Response> {
        use super::super::transport::Response;
        if self.offline.load(Ordering::Relaxed) { return Err(io::Error::other("offline")); }
        self.calls.lock().unwrap().push(format!("{path}:{range:?}"));
        if path == "properties" {
            let revision = self.changed_discovery.load(Ordering::Relaxed);
            return Ok(Response { bytes: format!("obs_collection=gaia\ndataproduct_type=object\nrevision={revision}\n").into_bytes(), identity: None, start: 0 });
        }
        if path == "partition_info.csv" {
            return Ok(Response { bytes: (String::from("Norder,Npix\n") + &(0..self.leaves).map(|n| format!("{},{n}\n", u8::from(self.leaves > 12))).collect::<String>()).into_bytes(), identity: None, start: 0 });
        }
        let id = Identity { etag: format!("\"{path}:{}\"", self.changed.load(Ordering::Relaxed)), size: self.data.len() as u64 };
        if identity.is_some_and(|expected| expected != &id) { return Err(io::Error::other("changed")); }
        let range = range.unwrap().strip_prefix("bytes=").unwrap();
        let (start, end) = range.split_once('-').unwrap();
        let (start, end) = if start.is_empty() {
            (self.data.len().saturating_sub(end.parse().unwrap()), self.data.len() - 1)
        } else { (start.parse().unwrap(), end.parse().unwrap()) };
        let bytes = self.data[start..=end].to_vec();
        assert!(bytes.len() <= limit);
        Ok(Response { bytes, identity: Some(id), start: start as u64 })
    }
}

#[test]
fn mast_prefix_extension_shrink_and_offline_reuse_only_fetch_missing_groups() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut times = StepTimes::default();
    let first = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut times).unwrap();
    assert_eq!((first.blocks.len(), first.blocks[0].selected, first.available), (1, 2, 3));
    let calls = server.calls.lock().unwrap().len();

    server.offline.store(true, Ordering::Relaxed);
    for count in [1, 2, 3] {
        assert_eq!(resolve_selection(count, 2, store.clone(), &server, &mut progress, &mut times).unwrap().blocks[0].selected, count as usize);
    }
    assert_eq!(server.calls.lock().unwrap().len(), calls);
    assert!(resolve_selection(4, 2, store.clone(), &server, &mut progress, &mut times).is_err());

    server.offline.store(false, Ordering::Relaxed);
    let next = resolve_selection(4, 2, store.clone(), &server, &mut progress, &mut times).unwrap();
    assert_eq!((next.blocks.len(), next.blocks[1].selected, next.blocks[1].base), (2, 1, 3));
    assert_eq!(next.blocks[0].hash, first.blocks[0].hash);
    assert!(server.calls.lock().unwrap()[calls..].iter().all(|call| call.contains("Npix=1")));
    server.offline.store(true, Ordering::Relaxed);
    let all = resolve_selection(99, 2, store, &server, &mut progress, &mut times).unwrap();
    assert_eq!(all.available, 6);
    assert!(String::from_utf8(progress.0).unwrap().contains("contains only 6"));
}

#[test]
fn mast_projection_preserves_nulls_exact_ids_and_nan_bits() {
    let mut a = row((1 << 54) + 1);
    a.values[2] = Some(2016f64.to_bits());
    a.values[3] = None;
    let mut b = row(77);
    b.values[0] = Some(f64::NAN.to_bits());
    b.values[1] = Some((-0.0f64).to_bits());
    let raw = parquet::write_block(&[a.clone(), b.clone()], schema()).unwrap();
    assert_eq!(parquet::read_block(raw).unwrap(), vec![a, b]);
}

#[test]
fn mast_discovery_rejects_bad_and_overlapping_leaves() {
    for bad in ["Norder,Npix\n0,0\n0,0\n", "Norder,Npix\n0,0\n1,1\n", "Norder,Npix\n30,0\n", "Norder,Npix\n0,12\n", "wrong,Npix\n0,0\n"] {
        assert!(parse_leaves(bad).is_err(), "{bad}");
    }
    assert_eq!(parse_leaves("Norder,Npix\n1,5\n0,0\n").unwrap(), vec![Leaf { order: 0, pixel: 0 }, Leaf { order: 1, pixel: 5 }]);
}

#[test]
fn mast_range_reader_refuses_unfetched_columns_and_bad_footer() {
    let bytes = parquet::write_block(&[row(10)], schema()).unwrap();
    let length = bytes.len();
    assert!(parquet::parse_footer(&bytes[..length - 1]).is_err());
    let metadata = parquet::parse_footer(&bytes).unwrap();
    let ranges = parquet::selected_ranges(&metadata, 0, length as u64).unwrap();
    assert_eq!(ranges.len(), 12);
    let tail = u32::from_le_bytes(bytes[length - 8..length - 4].try_into().unwrap()) as usize + 8;
    let input = Ranges { size: length as u64, parts: vec![((length - tail) as u64, Bytes::copy_from_slice(&bytes[length - tail..]))] };
    assert!(parquet::read_group(input, 0).is_err());
}

#[test]
fn mast_corrupt_raw_block_repairs_independently_but_changed_remote_identity_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut times = StepTimes::default();
    let initial = resolve_selection(4, 2, store.clone(), &server, &mut progress, &mut times).unwrap();
    let broken = dir.path().join("objects").join(&initial.blocks[0].hash);
    std::fs::write(&broken, b"interrupted/corrupt local data").unwrap();
    server.changed.store(true, Ordering::Relaxed);
    let error = resolve_selection(4, 2, store.clone(), &server, &mut progress, &mut times).err().expect("changed source must fail");
    assert!(error.to_string().contains("changed"));
    server.changed.store(false, Ordering::Relaxed);
    let repaired = resolve_selection(4, 2, store, &server, &mut progress, &mut times).unwrap();
    assert_eq!(repaired.blocks[1].hash, initial.blocks[1].hash);
    assert_eq!(repaired.blocks[0].hash, initial.blocks[0].hash);
}

#[test]
fn mast_pinned_objects_repair_only_when_the_original_identity_is_available() {
    for discovery in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().into());
        let server = Server::new();
        let mut progress = NoProgress(Vec::new());
        let mut times = StepTimes::default();
        let initial = resolve_selection(1, 2, store.clone(), &server, &mut progress, &mut times).unwrap();
        let key = if discovery { "selected-discovery-v1".to_owned() }
            else { format!("footer-v1:{}:dataset/Norder=0/Dir=0/Npix=0.parquet", initial.discovery) };
        let original = store.read(&key).unwrap().unwrap();
        let path = dir.path().join("objects").join(digest(&original));
        std::fs::write(&path, b"corruption").unwrap();
        let changed = if discovery { &server.changed_discovery } else { &server.changed };
        changed.store(true, Ordering::Relaxed);
        assert!(resolve_selection(1, 2, store.clone(), &server, &mut progress, &mut times).is_err());
        assert!(store.read(&key).is_err());                                                        // the failed repair did not repin to the changed remote data
        changed.store(false, Ordering::Relaxed);
        let repaired = resolve_selection(1, 2, store.clone(), &server, &mut progress, &mut times).unwrap();
        assert_eq!(repaired.discovery, initial.discovery);
        assert_eq!(store.read(&key).unwrap().unwrap(), original);
        assert!(String::from_utf8_lossy(&progress.0).contains(&path.display().to_string()));
        server.offline.store(true, Ordering::Relaxed);
        resolve_selection(1, 2, store, &server, &mut progress, &mut times).unwrap();
    }
}

#[test]
#[ignore = "local decode measurement; requires ASTROTERM_MAST_SAMPLE, no network"]
fn mast_measure_parquet_decode() {
    use std::{hint::black_box, time::Instant};
    let path = std::env::var_os("ASTROTERM_MAST_SAMPLE").expect("set a local normalized Parquet block");
    let input = std::fs::read(path).unwrap();
    for run in 0..3 {
        let bytes = input.clone();
        let started = Instant::now();
        let rows = parquet::read_block(bytes).unwrap();
        eprintln!("run={run}; rows={}; decode={:?}", rows.len(), started.elapsed());
        black_box(rows);
    }
}

#[derive(Default)]
struct BlockProgress { text: Vec<u8>, details: Vec<String>, totals: Vec<u64>, positions: Vec<u64>, cached: Vec<u64>, fail_at: Option<u64> }
impl std::io::Write for BlockProgress {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.text.extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DatasetProgress for BlockProgress {
    fn set_detail(&mut self, detail: &str) { self.details.push(detail.into()); }
    fn set_cached_blocks(&mut self, ready: u64) { self.cached.push(ready); self.set_position(ready); }
    fn begin_stage(&mut self, stage: Stage, _: &str, unit: Unit, total: Option<u64>) {
        if stage == Stage::Download {
            assert_eq!(unit, Unit::Blocks);
            self.totals.push(total.unwrap());
        }
    }
    fn set_position(&mut self, ready: u64) {
        if self.fail_at == Some(ready) { panic!("injected progress panic"); }
        self.positions.push(ready);
    }
}

/// Hold the first request wave until every block has filled its request slots. Timeouts turn regressions into failures.
struct OverlappingServer {
    source: Server,
    gate: std::sync::Condvar,
    started: Mutex<usize>,
    required: usize,
    active: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
    fail: bool,
    panic: bool,
}
impl OverlappingServer {
    fn new(parallel: u8) -> Self {
        let mut source = Server::new();
        source.leaves = 16;
        Self { source, gate: std::sync::Condvar::new(), started: Mutex::new(0), required: usize::from(parallel) * crate::catalog::mast::COLUMNS.len(),
            active: 0.into(), peak: 0.into(), fail: false, panic: false }
    }
}
impl Fetch for OverlappingServer {
    fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<super::super::transport::Response> {
        if identity.is_none() { return self.source.fetch(path, range, identity, limit, deadline); }
        struct Active<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Drop for Active<'_> { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _active = Active(&self.active);
        self.peak.fetch_max(active, Ordering::SeqCst);
        let mut started = self.started.lock().unwrap();
        *started += 1;
        let first = *started == 1;
        self.gate.notify_all();
        let (started, _) = self.gate.wait_timeout_while(started, Duration::from_secs(5), |count| *count < self.required).unwrap();
        let overlapped = *started >= self.required;
        drop(started);
        if !overlapped { return Err(io::Error::other("block downloads did not overlap")); }
        if first && self.panic { panic!("injected request panic"); }
        if first && self.fail { return Err(io::Error::other("injected download error")); }
        self.source.fetch(path, range, identity, limit, deadline)
    }
}

#[test]
fn mast_parallel_limits_preserve_order_and_one_monotonic_block_bar() {
    let mut expected = None;
    for parallel in [1, 2, 4, 8, 16] {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path().into());
        let server = OverlappingServer::new(parallel);
        let mut progress = BlockProgress::default();
        let selection = resolve_selection(47, parallel, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
        let outputs: Vec<_> = selection.blocks.iter().map(|block| (block.hash.clone(), block.base, block.selected)).collect();
        if let Some(expected) = &expected { assert_eq!(&outputs, expected); } else { expected = Some(outputs); }
        assert_eq!(server.active.load(Ordering::SeqCst), 0);
        assert_eq!(server.peak.load(Ordering::SeqCst), server.required);
        assert_eq!(progress.totals, vec![16]);
        assert_eq!(progress.positions, (0..=16).collect::<Vec<_>>());
        assert_eq!(selection.blocks.last().unwrap().selected, 2);
        server.source.offline.store(true, Ordering::SeqCst);
        let mut cached = BlockProgress::default();
        let reused = resolve_selection(47, 1, store, &server, &mut cached, &mut StepTimes::default()).unwrap();
        assert_eq!(reused.blocks.len(), 16);
        assert!(cached.totals.is_empty());                                                       // changing concurrency does not force new downloads
    }
}

#[test]
fn mast_worker_errors_panics_and_coordinator_panics_join_every_request() {
    for (fail, panic, progress_panic) in [(true, false, false), (false, true, false), (false, false, true)] {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path().into());
        let mut server = OverlappingServer::new(2);
        server.fail = fail;
        server.panic = panic;
        let mut progress = BlockProgress { fail_at: progress_panic.then_some(1), ..Default::default() };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_selection(35, 2, store.clone(), &server, &mut progress, &mut StepTimes::default())
        }));
        assert_eq!(server.active.load(Ordering::SeqCst), 0);
        if progress_panic { assert!(result.is_err()); }
        else {
            let error = result.unwrap().err().unwrap().to_string();
            assert!(error.contains(if panic { "range worker panicked" } else { "injected download error" }), "{error}");
        }
        let mut recovered = BlockProgress::default();
        let complete = resolve_selection(35, 2, store, &server.source, &mut recovered, &mut StepTimes::default()).unwrap();
        assert_eq!(complete.blocks.len(), 12);                                                   // restart fills holes instead of discarding completed blocks
        if progress_panic { assert!(recovered.positions[0] >= 1); }
    }
}

#[test]
fn mast_zero_does_not_touch_storage_or_network() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("must-not-exist");
    let server = Server::new();

    assert!(resolve_selection(0, 2, BlobStore::new(root.clone()), &server, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap().blocks.is_empty());
    assert!(!root.exists());
    assert!(server.calls.lock().unwrap().is_empty());
}

fn prepare_test_plan(store: &BlobStore, source: &Server, count: u32) -> super::super::downloading::Plan {
    let mut progress = NoProgress(Vec::new());
    let discovery = load_discovery(store, source, &mut progress).unwrap();
    let mut selection = MastSelection { store: store.clone(), blocks: Vec::new(), discovery: digest(&encode(&discovery).unwrap()), requested: count, available: 0 };
    plan_blocks(&mut selection, parse_leaves(&discovery.partitions).unwrap(), source, &mut progress, &mut StepTimes::default()).unwrap()
}

#[test]
fn mast_byte_budget_limits_outstanding_blocks_without_losing_results() {
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let server = OverlappingServer::new(1);                                                     // budget must reduce an eight-block setting to one active block
    let mut plan = prepare_test_plan(&store, &server.source, 12);
    let budget = plan.jobs.iter().map(|job| job.compressed).max().unwrap();
    super::super::downloading::run_with_budget(&store, &mut plan, 8, budget, &server, &mut BlockProgress::default(), &mut StepTimes::default()).unwrap();
    assert!(plan.ready.iter().all(Option::is_some));
    assert_eq!(server.peak.load(Ordering::SeqCst), crate::catalog::mast::COLUMNS.len());

    let mut invalid = prepare_test_plan(&store, &server.source, 15);
    assert!(super::super::downloading::run_with_budget(&store, &mut invalid, 8, 0, &server, &mut BlockProgress::default(), &mut StepTimes::default()).is_err());
}

#[test]
fn mast_slow_first_block_does_not_hold_up_publication_or_new_downloads() {
    let gate = std::sync::Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    struct SlowFirst { source: Server, gate: std::sync::Arc<(Mutex<bool>, std::sync::Condvar)> }
    impl Fetch for SlowFirst {
        fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<super::super::transport::Response> {
            if identity.is_some() && path.ends_with("Npix=0.parquet") {
                let (released, _) = self.gate.1.wait_timeout_while(self.gate.0.lock().unwrap(), Duration::from_secs(5), |ready| !*ready).unwrap();
                if !*released { return Err(io::Error::other("scheduler waited for the slow first block")); }
            }
            self.source.fetch(path, range, identity, limit, deadline)
        }
    }
    struct Published { gate: std::sync::Arc<(Mutex<bool>, std::sync::Condvar)>, store: BlobStore, first_key: String }
    impl std::io::Write for Published {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Published {
        fn set_position(&mut self, count: u64) {
            if count == 2 {
                assert!(self.store.read(&self.first_key).unwrap().is_none());                    // two later blocks were installed while the first still waits
                *self.gate.0.lock().unwrap() = true;
                self.gate.1.notify_all();
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let mut server = SlowFirst { source: Server::new(), gate: gate.clone() };
    server.source.leaves = 3;
    let mut plan = prepare_test_plan(&store, &server.source, 9);
    let mut progress = Published { gate, store: store.clone(), first_key: plan.jobs[0].key.clone() };
    super::super::downloading::download_missing(&store, &mut plan, 2, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(plan.ready.iter().map(|block| block.as_ref().unwrap().base).collect::<Vec<_>>(), [0, 3, 6]);
}

#[test]
fn mast_normalization_and_publication_failures_do_not_publish_partial_blocks() {
    for corrupt_input in [false, true] {
        struct BrokenSource { source: Server, corrupt: bool }
        impl Fetch for BrokenSource {
            fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<super::super::transport::Response> {
                let mut response = self.source.fetch(path, range, identity, limit, deadline)?;
                if self.corrupt && identity.is_some() { response.bytes.fill(0); }
                Ok(response)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path().into());
        let server = BrokenSource { source: Server::new(), corrupt: corrupt_input };
        let mut plan = prepare_test_plan(&store, &server.source, 4);
        if !corrupt_input {
            let reference = directory.path().join("references").join(format!("{}.toml", digest(plan.jobs[0].key.as_bytes())));
            std::fs::create_dir(reference).unwrap();                                            // publication cannot replace a directory with the completed reference
        }
        assert!(super::super::downloading::download_missing(&store, &mut plan, 2, &server, &mut BlockProgress::default(), &mut StepTimes::default()).is_err());
        assert!(plan.ready[0].is_none());
    }
}

#[test]
fn mast_reports_received_bytes_before_a_block_finishes() { check_streaming_progress(false); }

#[test]
fn mast_repair_reports_received_bytes_without_resetting_loading_progress() { check_streaming_progress(true); }

fn check_streaming_progress(repair: bool) {
    use super::super::transport::{TransferControl, Response};
    let gate = std::sync::Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    struct Streaming { source: Server, gate: std::sync::Arc<(Mutex<bool>, std::sync::Condvar)> }
    impl Fetch for Streaming {
        fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
            self.source.fetch(path, range, identity, limit, deadline)
        }
        fn fetch_cancellable(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant, control: &TransferControl) -> io::Result<Response> {
            if identity.is_none() { return self.source.fetch(path, range, identity, limit, deadline); } // metadata is separate from the block-byte progress being tested
            let response = self.source.fetch(path, range, identity, limit, deadline)?;
            control.add_received(response.bytes.len());
            let (released, _) = self.gate.1.wait_timeout_while(self.gate.0.lock().unwrap(), Duration::from_secs(5), |ready| !*ready).unwrap();
            if !*released { return Err(io::Error::other("no speed update arrived before block completion")); }
            Ok(response)
        }
    }
    struct RateProgress { gate: std::sync::Arc<(Mutex<bool>, std::sync::Condvar)>, ready: u64, bytes: u64, updated_while_waiting: bool }
    impl std::io::Write for RateProgress {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for RateProgress {
        fn set_position(&mut self, ready: u64) { self.ready = ready; }
        fn set_download_bytes(&mut self, bytes: u64, _: Duration) {
            assert!(bytes >= self.bytes);
            self.bytes = bytes;
            if bytes > 0 && self.ready == 0 {
                self.updated_while_waiting = true;
                *self.gate.0.lock().unwrap() = true;
                self.gate.1.notify_all();
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let source = Streaming { source: Server::new(), gate: gate.clone() };
    let metadata = parquet::parse_footer(&source.source.data).unwrap();
    let expected: u64 = parquet::selected_ranges(&metadata, 0, source.source.data.len() as u64).unwrap().iter().map(|(_, n)| *n as u64).sum();
    let mut progress = RateProgress { gate, ready: 0, bytes: 0, updated_while_waiting: false };
    if repair {
        let mut selection = resolve_selection(3, 2, store.clone(), &source.source, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap();
        let block = &mut selection.blocks[0];
        std::fs::write(directory.path().join("objects").join(&block.hash), b"corrupt").unwrap();
        super::super::downloading::read_or_repair_with_fetch(&store, block, &source, &mut progress, &mut StepTimes::default()).unwrap();
    } else {
        resolve_selection(3, 2, store, &source, &mut progress, &mut StepTimes::default()).unwrap();
    }
    assert!(progress.updated_while_waiting);
    assert_eq!(progress.ready, u64::from(!repair));
    assert_eq!(progress.bytes, expected);                                                       // metadata and cache reads do not inflate the download rate
}

#[test]
fn mast_progress_identifies_cached_blocks_before_new_downloads() {
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let server = Server::new();
    resolve_selection(3, 2, store.clone(), &server, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap();
    let mut progress = BlockProgress::default();
    resolve_selection(6, 2, store, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(progress.totals, vec![2]);
    assert_eq!(progress.cached, vec![1]);
    assert_eq!(progress.positions, vec![1, 2]);
}

#[test]
fn mast_planning_does_not_read_raw_payloads_even_if_they_are_damaged() {
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let selected = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    let block = &selected.blocks[0];
    let path = root.path().join("objects").join(&block.hash);
    let before = std::fs::metadata(&path).unwrap();
    std::fs::write(&path, vec![0; before.len() as usize]).unwrap();
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(before.modified().unwrap()).unwrap();
    let reads = store.payload_reads(&block.hash);
    server.offline.store(true, Ordering::Relaxed);
    let next = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(next.blocks[0].hash, block.hash);
    assert_eq!(store.payload_reads(&block.hash), reads);
    next.check_unchanged().unwrap();
}

#[test]
fn consumed_raw_repairs_once_under_the_pin_and_refreshes_its_stamp() {
    use super::super::downloading::read_or_repair_with_fetch;
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut selected = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    let block = &mut selected.blocks[0];
    let hash = block.hash.clone();
    let path = root.path().join("objects").join(&hash);
    let size = std::fs::metadata(&path).unwrap().len();
    std::fs::write(&path, vec![0; size as usize]).unwrap();
    let calls = server.calls.lock().unwrap().len();
    let bytes = read_or_repair_with_fetch(&store, block, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(block.hash, hash);
    assert_eq!(parquet::read_block(bytes).unwrap(), [row(10), row(20), row(30)]);
    assert_eq!(server.calls.lock().unwrap().len() - calls, super::super::COLUMNS.len());
    selected.check_unchanged().unwrap();

    std::fs::write(&path, vec![0; size as usize]).unwrap();
    server.changed.store(true, Ordering::Relaxed);
    let block = &mut selected.blocks[0];
    let stamp = block.stamp;
    let error = read_or_repair_with_fetch(&store, block, &server, &mut progress, &mut StepTimes::default()).unwrap_err();
    assert!(error.to_string().contains("changed"));
    assert_eq!((&block.hash, block.stamp), (&hash, stamp));
}

#[test]
fn repair_can_replace_an_older_writers_hash_without_replanning() {
    use super::super::downloading::read_or_repair_with_fetch;
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut selected = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    let block = &mut selected.blocks[0];
    let normalized_hash = block.hash.clone();
    let mut older = store.read_object(&block.hash).unwrap();
    let producer = ::parquet::file::properties::DEFAULT_CREATED_BY.as_bytes();
    let offset = older.windows(producer.len()).rposition(|bytes| bytes == producer).unwrap();
    older[offset] = b'x';                                    // change only the harmless writer marker in valid Parquet metadata
    assert_eq!(parquet::read_block(older.clone()).unwrap(), [row(10), row(20), row(30)]);
    let old = store.publish(&block.key, &older, false).unwrap();
    block.hash = old.hash.clone();
    block.stamp = old.stamp;
    std::fs::write(root.path().join("objects").join(&old.hash), vec![0; older.len()]).unwrap();
    read_or_repair_with_fetch(&store, block, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(block.hash, normalized_hash);
    assert_ne!(block.hash, old.hash);
    assert_eq!(store.locate(&block.key).unwrap().unwrap().hash, normalized_hash);
    selected.check_unchanged().unwrap();
    assert!(root.path().join("objects").join(&old.hash).exists()); // no eviction or cleanup sweep
}

#[test]
fn missing_consumed_object_is_repaired_but_a_changed_reference_is_not_followed() {
    use super::super::downloading::read_or_repair_with_fetch;
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut selected = resolve_selection(2, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
    let block = &mut selected.blocks[0];
    let expected = block.hash.clone();
    store.publish(&block.key, b"unrelated concurrent reference", false).unwrap();
    let calls = server.calls.lock().unwrap().len();
    let raw = read_or_repair_with_fetch(&store, block, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(block.hash, expected);
    assert_eq!(server.calls.lock().unwrap().len(), calls);
    assert_eq!(parquet::read_block(raw).unwrap(), [row(10), row(20), row(30)]);
    std::fs::remove_file(root.path().join("objects").join(&expected)).unwrap(); // isolated test fixture: simulate disappearance after planning
    read_or_repair_with_fetch(&store, block, &server, &mut progress, &mut StepTimes::default()).unwrap();
    assert_eq!(block.hash, expected);
    assert_eq!(server.calls.lock().unwrap().len() - calls, super::super::COLUMNS.len());
    selected.check_unchanged().unwrap();
}

#[test]
fn planning_downloads_is_one_stage_across_missing_and_cached_leaf_metadata() {
    #[derive(Default)]
    struct Stages { stages: Vec<Stage>, details: Vec<String> }
    impl std::io::Write for Stages {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Stages {
        fn begin_stage(&mut self, stage: Stage, _: &str, _: Unit, _: Option<u64>) { self.stages.push(stage); }
        fn set_detail(&mut self, detail: &str) { self.details.push(detail.into()); }
    }
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let server = Server::new();
    for attempt in 0..2 {
        let mut progress = Stages::default();
        let selection = resolve_selection(6, 2, store.clone(), &server, &mut progress, &mut StepTimes::default()).unwrap();
        assert_eq!(selection.blocks.len(), 2);
        assert_eq!(progress.details.iter().filter(|detail| detail.starts_with("Waiting for file metadata:")).count(), if attempt == 0 { 2 } else { 0 });
        assert_eq!(progress.stages.iter().filter(|&&stage| stage == Stage::PlanDownloads).count(), 1);
        let checking = progress.stages.iter().position(|&stage| stage == Stage::PlanDownloads).unwrap();
        assert!(progress.stages[checking + 1..].iter().all(|&stage| stage == Stage::Download)); // fetching a new footer must not replace this stage
    }
}

#[test]
fn repair_cancels_and_joins_workers_when_progress_panics() {
    use super::super::transport::{TransferControl, Response};
    struct Waiting { source: Server, active: std::sync::atomic::AtomicUsize }
    impl Fetch for Waiting {
        fn fetch(&self, path: &str, range: Option<&str>, identity: Option<&Identity>, limit: usize, deadline: Instant) -> io::Result<Response> {
            self.source.fetch(path, range, identity, limit, deadline)
        }
        fn fetch_cancellable(&self, _: &str, _: Option<&str>, _: Option<&Identity>, _: usize, _: Instant, control: &TransferControl) -> io::Result<Response> {
            self.active.fetch_add(1, Ordering::Relaxed);
            control.add_received(1);
            let deadline = Instant::now() + Duration::from_secs(5);
            while control.check().is_ok() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(5)); }
            self.active.fetch_sub(1, Ordering::Relaxed);
            control.check()?;
            Err(io::Error::other("repair did not cancel"))
        }
    }
    struct Panicking;
    impl std::io::Write for Panicking {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Panicking {
        fn set_download_bytes(&mut self, bytes: u64, _: Duration) { if bytes > 0 { panic!("repair progress panic"); } }
    }
    let directory = tempfile::tempdir().unwrap();
    let store = BlobStore::new(directory.path().into());
    let server = Waiting { source: Server::new(), active: 0.into() };
    let mut selection = resolve_selection(3, 2, store.clone(), &server.source, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap();
    let block = &mut selection.blocks[0];
    std::fs::write(directory.path().join("objects").join(&block.hash), b"corrupt").unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::downloading::read_or_repair_with_fetch(&store, block, &server, &mut Panicking, &mut StepTimes::default())
    })).unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"repair progress panic"));
    assert_eq!(server.active.load(Ordering::Relaxed), 0);
    assert!(store.read(&block.key).is_err()); // failed repair must not publish a replacement
}

mod metadata;
