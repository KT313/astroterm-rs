//! Opt-in real transport qualification. Ordinary tests never download data.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, sky::load_sky_catalog_with_times,
    timing::StepTimes, progress::{DatasetProgress, Stage, Unit}};
use std::{io::{self, Write}, path::PathBuf, time::Duration};

#[derive(Default)]
struct Counts { downloaded: usize, converted: usize, text: Vec<u8>, downloading: bool, last_ready: Option<u64>, received: u64, transfer_elapsed: Duration }
impl Write for Counts {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.text.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DatasetProgress for Counts {
    fn begin_stage(&mut self, stage: Stage, detail: &str, unit: Unit, _: Option<u64>) {
        self.downloading = stage == Stage::Download;
        self.last_ready = None;
        if self.downloading { assert_eq!(unit, Unit::Blocks); }
        if stage == Stage::Convert && detail == "MAST block" { self.converted += 1; }
        eprintln!("{}: {detail}", stage.label());
    }
    fn set_download_bytes(&mut self, received: u64, elapsed: Duration) {
        assert!(received >= self.received);
        self.received = received;
        self.transfer_elapsed = elapsed;
    }
    fn set_position(&mut self, ready: u64) {
        if !self.downloading { return; }
        if let Some(previous) = self.last_ready { self.downloaded += (ready - previous) as usize; }
        self.last_ready = Some(ready);                                                          // the initial count represents already cached blocks
    }
}

#[test]
#[ignore = "requires explicit ASTROTERM_MAST_TEST_ROOT and network; at most the first two row groups"]
fn mast_two_groups_extend_without_reconverting_old_rows() {
    let root = PathBuf::from(std::env::var_os("ASTROTERM_MAST_TEST_ROOT").expect("set an isolated empty test data directory"));
    let directories = DatasetDirectories { data: Some(root.join("data")), cache: Some(root.join("cache")) };
    let mut first_ids = Vec::new();
    for (count, parallel, expected_downloads, expected_conversions) in [(200001, 2, 2, 2), (100000, 1, 0, 0), (150000, 4, 0, 0), (200001, 8, 0, 0)] {
        let mut log = Counts::default();
        let started = std::time::Instant::now();
        let catalog = load_sky_catalog_with_times(Some(&Dataset::GaiaGdr3Stars { stars: count }), parallel, &directories, &mut log, &mut StepTimes::default()).unwrap();
        assert_eq!((log.downloaded, log.converted), (expected_downloads, expected_conversions));
        assert_eq!(log.received > 0, expected_downloads > 0);
        let mut ids: Vec<_> = catalog.catalog.stars.iter().map(|star| star.id.0).collect();
        ids.sort_unstable();
        if first_ids.is_empty() { first_ids = ids.clone(); }
        else { assert!(ids.iter().all(|id| first_ids.binary_search(id).is_ok())); }
        assert!(catalog.catalog.stars.iter().all(|star| catalog.catalog.names.get(star.name).unwrap().starts_with("Gaia DR3 ")));
        if log.received > 0 {
            eprintln!("Received {:.2} MiB in {:.3}s of block downloading/normalization/publication ({:.2} MiB/s avg)",
                log.received as f64 / 1_048_576.0, log.transfer_elapsed.as_secs_f64(), log.received as f64 / 1_048_576.0 / log.transfer_elapsed.as_secs_f64());
        }
        eprintln!("N={count}; parallel={parallel}; loaded={}; download blocks={}; converted blocks={}; elapsed={:?}\n{}",
            ids.len(), log.downloaded, log.converted, started.elapsed(), String::from_utf8_lossy(&log.text));
    }
}
