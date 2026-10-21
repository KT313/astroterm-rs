//! Progress follows actual loading work; recording it must not change the prepared catalog.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, progress::{DatasetProgress, Stage, Unit, Outcome}, sky::load_sky_catalog, time::StepTimes};
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
    let actual = load_sky_catalog(None, astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut progress, &mut StepTimes::default())).unwrap();
    let expected = load_sky_catalog(None, astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(Vec::new()), &mut StepTimes::default())).unwrap();
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
        let actual = load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut first, &mut StepTimes::default())).unwrap();
        assert_eq!(actual.catalog.stars.len(), 2);
        assert!(first.stages.contains(&Stage::Parse) && first.stages.contains(&Stage::WriteCache));
        assert!(first.positions.contains(&3));
        let mut hit = Recording::default();
        let cached = load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut hit, &mut StepTimes::default())).unwrap();
        assert_eq!(actual, cached);
        assert!(hit.stages.contains(&Stage::ReadCache) && hit.stages.contains(&Stage::ValidateCache));
        assert!(!hit.stages.contains(&Stage::Parse) && !hit.stages.contains(&Stage::Sort));
    }
}

#[test]
fn zero_gaia_keeps_its_notice_without_fake_download_or_rows() {
    let mut reporter = Recording::default();
    let catalog = load_sky_catalog(Some(&Dataset::GaiaGdr3 { files: 0 }), astroterm::constants::MAST_PARALLEL_BLOCKS, &DatasetDirectories::default(), &mut astroterm::progress::Report::live(&mut reporter, &mut StepTimes::default())).unwrap();
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
    assert!(load_sky_catalog(Some(&Dataset::Path(path.clone())), astroterm::constants::MAST_PARALLEL_BLOCKS, &DatasetDirectories::default(), &mut astroterm::progress::Report::live(&mut reporter, &mut StepTimes::default())).is_err());
    assert_eq!(reporter.outcomes.last(), Some(&Outcome::Failed));
    std::fs::write(&path, b"ra,dec,mag\n0,0,5\n").unwrap();
    let cache_file = dir.path().join("cache-is-a-file");
    std::fs::write(&cache_file, b"cannot create a directory here").unwrap();
    let dirs = DatasetDirectories { data: None, cache: Some(cache_file) };
    let mut reporter = Recording::default();
    let catalog = load_sky_catalog(Some(&Dataset::Path(path)), astroterm::constants::MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut reporter, &mut StepTimes::default())).unwrap();
    assert_eq!(catalog.catalog.stars.len(), 1);
    assert_eq!(reporter.outcomes.last(), Some(&Outcome::Complete));
    assert!(String::from_utf8_lossy(&reporter.text).contains("Could not write cache"));
}

#[test]
fn zero_mast_does_not_need_storage_or_network_and_reports_gaia_policy() {
    let mut reporter=Recording::default();
    let catalog=load_sky_catalog(Some(&Dataset::GaiaGdr3Stars{stars:0}), astroterm::constants::MAST_PARALLEL_BLOCKS, &DatasetDirectories::default(), &mut astroterm::progress::Report::live(&mut reporter, &mut StepTimes::default())).unwrap();
    assert!(catalog.catalog.stars.is_empty());
    assert!(!reporter.stages.contains(&Stage::Connect));
    assert!(!reporter.stages.contains(&Stage::Download));
    assert_eq!(String::from_utf8(reporter.text).unwrap().matches("no G-to-V conversion").count(),1);
}

#[test]
fn invalid_mast_parallel_limits_fail_at_the_loader_boundary_even_for_zero_rows() {
    for count in [0, 1] {
        for parallel in [0, 17, 255] {
            let mut progress = Recording::default();
            let error = load_sky_catalog(Some(&Dataset::GaiaGdr3Stars { stars: count }), parallel, &DatasetDirectories::default(), &mut astroterm::progress::Report::live(&mut progress, &mut StepTimes::default())).err().unwrap();
            assert!(error.to_string().contains("between 1 and 16"));
            assert!(progress.stages.is_empty());                                                // validation precedes directory lookup and network access
        }
    }
}

#[derive(Default)]
struct StageAudit {
    active: Option<(Stage, u64, Option<u64>)>,
    starts: Vec<Stage>,
    completed: Vec<Stage>,
    bytes: Vec<(u64, u64)>,
}
impl Write for StageAudit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DatasetProgress for StageAudit {
    fn begin_stage(&mut self, stage: Stage, _: &str, _: Unit, total: Option<u64>) {
        self.starts.push(stage);
        self.active = Some((stage, 0, total));
    }
    fn set_work(&mut self, _: Unit, total: Option<u64>) {
        if let Some((_, _, known)) = &mut self.active { *known = total; }
    }
    fn set_position(&mut self, position: u64) {
        if let Some((_, previous, total)) = &mut self.active {
            assert!(position >= *previous, "one stage must not reset its counter");
            assert!(total.is_none_or(|total| position <= total), "counter exceeds real work");
            *previous = position;
        }
    }
    fn set_read_progress(&mut self, bytes: u64, total: u64) { assert!(bytes <= total); self.bytes.push((bytes, total)); }
    fn finish_stage(&mut self, outcome: Outcome) {
        if let Some((stage, position, total)) = self.active.take() && outcome == Outcome::Complete {
            if let Some(total) = total { assert_eq!(position, total, "completed {stage:?} before the work finished"); }
            self.completed.push(stage);
        }
    }
}

#[test]
fn preparation_and_cache_traces_cover_visible_work_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("stars.csv");
    std::fs::write(&source, b"ra,dec,mag\n0,0,5\n1,1,6\n").unwrap();
    let dataset = Dataset::Path(source);
    let directories = DatasetDirectories { data: None, cache: Some(dir.path().join("cache")) };
    let mut progress = StageAudit::default();
    let mut times = StepTimes::with_trace(true);
    let original = load_sky_catalog(Some(&dataset), 4, &directories, &mut astroterm::progress::Report::live(&mut progress, &mut times)).unwrap();
    for pair in [Stage::AssignRegions, Stage::Sort, Stage::Reorder, Stage::Index, Stage::ValidateCache, Stage::WriteCache, Stage::FinalizeCache].windows(2) {
        assert!(progress.starts.iter().position(|stage| stage == &pair[0]).unwrap() < progress.starts.iter().position(|stage| stage == &pair[1]).unwrap());
    }
    times.measure("END");
    let trace = times.trace().unwrap();
    for name in ["Region assignment", "Region and brightness sort", "Star field permutation", "Sky index construction",
        "Prepared catalog write", "Prepared star validation", "Prepared ID and region validation", "Prepared section writing", "Prepared catalog finalization"] {
        assert!(trace.steps.iter().any(|step| step.name == name), "missing {name}");
    }
    assert_eq!(trace.steps.iter().filter(|step| step.name == "Prepared catalog write").count(), 1);

    let mut hit = StageAudit::default();
    let mut times = StepTimes::with_trace(true);
    let cached = load_sky_catalog(Some(&dataset), 4, &directories, &mut astroterm::progress::Report::live(&mut hit, &mut times)).unwrap();
    assert_eq!(original, cached);
    assert_eq!(hit.starts, [Stage::Resolve, Stage::ReadCache, Stage::ValidateCache]);
    assert_eq!(hit.completed, [Stage::ReadCache, Stage::ValidateCache]);
    assert!(times.trace().unwrap().steps.iter().any(|step| step.name == "Prepared section reading"));
}

#[test]
fn source_and_cached_loads_report_actual_work_and_ordered_skip_reasons() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("small.csv");
    std::fs::write(&path, "ra,dec,mag\n0,0,4\n1,2,5\n,0,-26\n1,,4\n1,2,\n0,0,-26\n").unwrap();
    let dirs = DatasetDirectories { data: None, cache: Some(temp.path().join("cache")) };
    for warm in [false, true] {
        let mut times = StepTimes::with_trace(true);
        let catalog = load_sky_catalog(Some(&Dataset::Path(path.clone())), astroterm::constants::MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(Vec::new()), &mut times)).unwrap();
        assert_eq!(catalog.catalog.stars.len(), 2);
        let mut report = Vec::new();
        times.measure("END");
        times.trace().unwrap().write_report(&mut report).unwrap();
        let report = String::from_utf8(report).unwrap();
        if warm {
            assert!(report.contains("hit: validated owned stars=2"));
            assert!(report.contains("source CSV not read; original skipped-row counts unavailable"));
            assert!(!report.contains("CSV validation and parsing:"));
        } else {
            assert!(report.contains("input rows=6; removed missing RA/Dec/magnitude=3; then removed Sun (mag < -20)=1; output stars=2"));
            assert!(report.find("CSV validation and parsing:").unwrap() < report.find("Catalog preparation:").unwrap());
        }
    }
}
