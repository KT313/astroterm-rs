use super::*;
use crate::progress::NoProgress;
use std::io::Cursor;

fn entry(index: u32, bytes: &[u8]) -> GaiaEntry {
    GaiaEntry { name: format!("GaiaSource_{index:06}-{index:06}.csv.gz"), checksum: Md5::digest(bytes).into(), first_cell: index, last_cell: index }
}

#[test]
fn pinned_manifest_has_stable_prefix_and_disjoint_valid_ranges() {
    let entries = parse_manifest(MANIFEST, 3386).unwrap();
    assert_eq!(entries[0].name, "GaiaSource_000000-003111.csv.gz");
    assert_eq!(entries[4].name, "GaiaSource_007953-010234.csv.gz");
    assert_eq!(entries.last().unwrap().last_cell, 786431);
    let reversed = MANIFEST.lines().rev().collect::<Vec<_>>().join("\n");
    assert_eq!(parse_manifest(&reversed, 3386).unwrap()[0].name, entries[0].name);
    for text in ["bad file", "00000000000000000000000000000000 ../escape",
        "00000000000000000000000000000000 GaiaSource_000001-000000.csv.gz"] {
        assert!(parse_manifest(text, 1).is_err());
    }
    let duplicate = format!("{}\n{}", MANIFEST.lines().next().unwrap(), MANIFEST.lines().next().unwrap());
    assert!(parse_manifest(&duplicate, 2).is_err());
}

#[test]
fn increasing_count_fetches_only_missing_files_and_smaller_prefix_is_offline() {
    let dir = tempfile::tempdir().unwrap();
    let entries: Vec<_> = (0..8).map(|i| entry(i, &[i as u8])).collect();
    let mut requests = Vec::new();
    let mut fetch = |entry: &GaiaEntry| {
        requests.push(entry.first_cell);
        Ok((Box::new(Cursor::new(vec![entry.first_cell as u8])) as Box<dyn Read>, Some(1)))
    };
    resolve_entries(&entries[..5], dir.path(), &mut NoProgress(Vec::new()), &mut StepTimes::default(), &mut fetch).unwrap();
    let all = resolve_entries(&entries, dir.path(), &mut NoProgress(Vec::new()), &mut StepTimes::default(), &mut fetch).unwrap();
    assert_eq!(requests, (0..8).collect::<Vec<_>>());
    assert_eq!(all.len(), 8);
    let smaller = resolve_entries(&entries[..3], dir.path(), &mut NoProgress(Vec::new()), &mut StepTimes::default(), |_| panic!("offline")).unwrap();
    assert_eq!(smaller.len(), 3);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 8);
}

#[test]
fn corrupted_existing_file_is_replaced_only_by_verified_complete_data() {
    let dir = tempfile::tempdir().unwrap();
    let entry = entry(0, b"good");
    let path = dir.path().join(&entry.name);
    fs::write(&path, b"bad").unwrap();
    assert!(install_verified(&b"short"[..], &path, entry.checksum, Some(10), &mut NoProgress(Vec::new())).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"bad");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    resolve_entries(&[entry], dir.path(), &mut NoProgress(Vec::new()), &mut StepTimes::default(), |_| Ok((Box::new(Cursor::new(b"good")), Some(4)))).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"good");
}

#[test]
fn interrupted_and_oversized_downloads_leave_no_final_file() {
    struct Broken;
    impl Read for Broken { fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::Error::other("interrupted")) } }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.gz");
    let checksum = Md5::digest(b"good").into();
    assert!(install_verified(Broken, &path, checksum, None, &mut NoProgress(Vec::new())).is_err());
    assert!(install_verified(&b"good"[..], &path, checksum, Some(2), &mut NoProgress(Vec::new())).is_err());
    assert!(install_verified(&b"good"[..], &path, checksum, Some(GAIA_DOWNLOAD_MAX_BYTES + 1), &mut NoProgress(Vec::new())).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn concurrent_complete_installations_preserve_valid_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.gz");
    let bytes = vec![42; 1000];
    let checksum = Md5::digest(&bytes).into();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| install_verified(bytes.as_slice(), &path, checksum, None, &mut NoProgress(Vec::new())));
        let second = scope.spawn(|| install_verified(bytes.as_slice(), &path, checksum, None, &mut NoProgress(Vec::new())));
        first.join().unwrap().unwrap(); second.join().unwrap().unwrap();
    });
    assert!(verify_file(&path, checksum, &mut NoProgress(io::sink())).unwrap().is_some());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn zero_requires_neither_data_directory_nor_network() {
    assert!(resolve_gaia_files(0, &DatasetDirectories::default(), &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap().is_empty());
    assert!(resolve_gaia_files(3387, &DatasetDirectories::default(), &mut NoProgress(Vec::new()), &mut StepTimes::default()).is_err());
}

#[test]
fn transfer_progress_handles_unknown_lengths_and_marks_only_verified_success() {
    #[derive(Default)]
    struct Recorder { bytes: Vec<u8>, stages: Vec<(Stage, Option<u64>)>, positions: Vec<u64>, outcomes: Vec<Outcome> }
    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.bytes.write(bytes) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl DatasetProgress for Recorder {
        fn begin_stage(&mut self, stage: Stage, _: &str, _: Unit, total: Option<u64>) { self.stages.push((stage, total)); }
        fn set_position(&mut self, value: u64) { self.positions.push(value); }
        fn finish_stage(&mut self, outcome: Outcome) { self.outcomes.push(outcome); }
    }
    let dir = tempfile::tempdir().unwrap();
    let checksum = Md5::digest(b"test").into();
    let path = dir.path().join("test.gz");
    let mut recorder = Recorder::default();
    install_verified(&b"test"[..], &path, checksum, None, &mut recorder).unwrap();
    assert!(recorder.stages.contains(&(Stage::Download, None)));
    assert_eq!(recorder.positions.last(), Some(&4));
    assert_eq!(recorder.outcomes.last(), Some(&Outcome::Complete));
    recorder = Recorder::default();
    assert!(verify_file(&path, checksum, &mut recorder).unwrap().is_some());
    assert_eq!(recorder.stages, [(Stage::Verify, Some(4))]);
    assert_eq!(recorder.positions.last(), Some(&4));
    recorder = Recorder::default();
    assert!(install_verified(&b"wrong"[..], &path, checksum, Some(5), &mut recorder).is_err());
    assert!(!recorder.outcomes.contains(&Outcome::Complete));
    assert_eq!(fs::read(&path).unwrap(), b"test");
}
