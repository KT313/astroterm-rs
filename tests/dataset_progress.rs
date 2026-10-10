//! Progress follows actual loading work; recording it must not change the prepared catalog.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, progress::{DatasetProgress, Stage, Unit, Outcome}, sky::{load_sky_catalog, load_sky_catalog_with_times}, timing::StepTimes};
use std::io::{self, Write};

#[derive(Default)]
struct Recording {
    text: Vec<u8>,
    stages: Vec<Stage>,
    totals: Vec<(Unit, Option<u64>)>,
    positions: Vec<u64>,
    outcomes: Vec<Outcome>,
}
impl Write for Recording {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.text.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DatasetProgress for Recording {
    fn begin_stage(&mut self, stage: Stage, _: &str, unit: Unit, total: Option<u64>) { self.stages.push(stage); self.totals.push((unit, total)); }
    fn set_position(&mut self, value: u64) { self.positions.push(value); }
    fn finish_stage(&mut self, value: Outcome) { self.outcomes.push(value); }
}

#[test]
fn embedded_preparation_reports_rows_and_preserves_results() {
    let directories = DatasetDirectories::default();
    let mut progress = Recording::default();
    let actual = load_sky_catalog_with_times(None, &directories, &mut progress, &mut StepTimes::default()).unwrap();
    let expected = load_sky_catalog(None, &directories, &mut Vec::new()).unwrap();
    assert_eq!(actual, expected);
    assert!(progress.stages.contains(&Stage::Parse) && progress.stages.contains(&Stage::Sort) && progress.stages.contains(&Stage::Index));
    assert!(progress.totals.contains(&(Unit::Rows, Some(9110))));
    assert!(progress.positions.contains(&9110));
}

#[test]
fn plain_and_gzip_inputs_and_cache_hits_report_only_work_performed() {
    let dir = tempfile::tempdir().unwrap();
    let directories = DatasetDirectories { data: None, cache: Some(dir.path().join("cache")) };
    let csv = b"ra,dec,mag\n0,0,5\n1,1,6\n,,\n";
    for gzip in [false, true] {
        let path = dir.path().join(if gzip { "tiny.csv.gz" } else { "tiny.csv" });
        if gzip {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(csv).unwrap(); std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        } else { std::fs::write(&path, csv).unwrap(); }
        let dataset = Dataset::Path(path);
        let mut first = Recording::default();
        let actual = load_sky_catalog_with_times(Some(&dataset), &directories, &mut first, &mut StepTimes::default()).unwrap();
        assert_eq!(actual.catalog.stars.len(), 2);
        assert!(first.stages.contains(&Stage::Parse) && first.stages.contains(&Stage::WriteCache));
        assert!(first.positions.contains(&3));
        let mut hit = Recording::default();
        let cached = load_sky_catalog_with_times(Some(&dataset), &directories, &mut hit, &mut StepTimes::default()).unwrap();
        assert_eq!(actual, cached);
        assert!(hit.stages.contains(&Stage::ReadCache) && hit.stages.contains(&Stage::ValidateCache));
        assert!(!hit.stages.contains(&Stage::Parse) && !hit.stages.contains(&Stage::Sort));
    }
}

#[test]
fn zero_gaia_keeps_its_notice_without_fake_download_or_rows() {
    let mut reporter = Recording::default();
    let catalog = load_sky_catalog_with_times(Some(&Dataset::GaiaGdr3 { files: 0 }), &DatasetDirectories::default(), &mut reporter, &mut StepTimes::default()).unwrap();
    assert!(catalog.catalog.stars.is_empty());
    assert!(!reporter.stages.contains(&Stage::Download));
    assert!(!reporter.stages.contains(&Stage::Parse));
    assert_eq!(String::from_utf8(reporter.text).unwrap().matches("no G-to-V conversion").count(), 1);
}

#[test]
fn failed_input_reports_failure_and_cache_write_failure_stays_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiny.csv");
    std::fs::write(&path, b"ra,dec,mag\n0,0,broken\n").unwrap();
    let mut reporter = Recording::default();
    assert!(load_sky_catalog_with_times(Some(&Dataset::Path(path.clone())), &DatasetDirectories::default(), &mut reporter, &mut StepTimes::default()).is_err());
    assert_eq!(reporter.outcomes.last(), Some(&Outcome::Failed));
    std::fs::write(&path, b"ra,dec,mag\n0,0,5\n").unwrap();
    let cache_file = dir.path().join("cache-is-a-file");
    std::fs::write(&cache_file, b"cannot create a directory here").unwrap();
    let dirs = DatasetDirectories { data: None, cache: Some(cache_file) };
    let mut reporter = Recording::default();
    let catalog = load_sky_catalog_with_times(Some(&Dataset::Path(path)), &dirs, &mut reporter, &mut StepTimes::default()).unwrap();
    assert_eq!(catalog.catalog.stars.len(), 1);
    assert_eq!(reporter.outcomes.last(), Some(&Outcome::Complete));
    assert!(String::from_utf8_lossy(&reporter.text).contains("Could not write cache"));
}
