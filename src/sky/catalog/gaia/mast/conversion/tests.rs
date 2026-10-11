use super::*;
use crate::catalog::MastSelection;
use crate::{catalog::StarNames, progress::NoProgress};

fn row(id: u64) -> MastRow {
    MastRow { source_id: id, values: [45., 0., 2016., 1., 2., 1., 20., 0., 12., 13., 11.].map(|value| Some(f64::to_bits(value))) }
}
fn selection(dir: &std::path::Path, groups: &[Vec<MastRow>], count: usize) -> (MastSelection, BlobStore, String) {
    let raw = BlobStore::new(dir.join("raw"));
    let converted = BlobStore::new(dir.join("converted"));
    let policy = fingerprint();
    let mut base = 0u64;
    let mut blocks = Vec::new();
    for (i, rows) in groups.iter().enumerate() {
        if base >= count as u64 { break; }
        let key = format!("block{i}");
        let blob = raw.publish(&key, &[i as u8], false).unwrap();                                 // intentionally not Parquet: reuse must not decode these source blobs
        let block = MastBlock { key, hash: blob.hash, rows: rows.len(), selected: rows.len().min(count - base as usize), base, stamp: blob.stamp, source: None };
        let key = block_key(&block, &policy);
        converted.publish(&key, &encode_mast(&convert_block(rows).unwrap()).unwrap(), false).unwrap();
        base += rows.len() as u64;
        blocks.push(block);
    }
    (MastSelection { store: raw, blocks, discovery: "test".into(), requested: count as u32, available: base }, converted, policy)
}

#[test]
fn mast_converted_prefix_reuses_results_and_deduplicates_across_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (row(10), row(20), row(30));
    let groups = vec![vec![a, b.clone()], vec![b.clone(), b, c]];
    for (count, expected) in [(1, vec![0]), (2, vec![0, 1]), (3, vec![0, 1]), (4, vec![0, 1]), (5, vec![0, 1, 4])] {
        let (mut selection, store, policy) = selection(dir.path(), &groups, count);
        let reads: Vec<_> = selection.blocks.iter().map(|block| selection.store.payload_reads(&block.hash)).collect();
        let mut progress = NoProgress(Vec::new());
        let catalog = super::super::assembly::assemble_selected_catalog(&mut selection, &store, &policy, &mut progress, &mut StepTimes::default()).unwrap();
        assert_eq!(reads, selection.blocks.iter().map(|block| selection.store.payload_reads(&block.hash)).collect::<Vec<_>>());
        let mut ids: Vec<_> = catalog.catalog.stars.iter().map(|star| star.id.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, expected);
        assert!(catalog.catalog.stars.source_ids().is_some());
        assert!(catalog.catalog.stars.name_entries().is_empty());
        if count == 5 { assert!(String::from_utf8(progress.0).unwrap().contains("duplicate copies removed=2")); }
    }
}

#[test]
fn mast_overfetched_error_is_deferred_until_selected_and_conflicts_fail() {
    let dir = tempfile::tempdir().unwrap();
    let mut bad = row(20);
    bad.values[0] = Some(f64::NAN.to_bits());
    let groups = vec![vec![row(10), bad]];
    for count in [1, 2] {
        let (mut selection, store, policy) = selection(dir.path(), &groups, count);
        let result = super::super::assembly::assemble_selected_catalog(&mut selection, &store, &policy, &mut NoProgress(Vec::new()), &mut StepTimes::default());
        assert_eq!(result.is_ok(), count == 1);
        if count == 2 { assert!(result.unwrap_err().to_string().contains("nonfinite")); }
    }
    let mut conflict = row(10);
    conflict.values[0] = Some(44f64.to_bits());
    let data = convert_block(&[row(10), conflict]).unwrap();
    assert!(matches!(data.entries[1].result, ConvertedRow::Error { .. }));
    let (mut selection, store, policy) = selection(dir.path(), &[vec![row(10), row(20), row(10)]], 3);
    let error = super::super::assembly::assemble_selected_catalog(&mut selection, &store, &policy, &mut NoProgress(Vec::new()), &mut StepTimes::default()).unwrap_err();
    assert!(error.to_string().contains("nonadjacent"));
}

#[test]
fn mast_duplicate_float_comparison_uses_bits_but_nan_still_fails_validation() {
    let mut nan = row(1);
    nan.values[0] = Some(0x7ff8_0000_0000_0042);
    let converted = convert_block(&[nan.clone(), nan]).unwrap();
    assert!(matches!(converted.entries[0].result, ConvertedRow::Error { .. }));
    assert!(matches!(converted.entries[1].result, ConvertedRow::Duplicate));
    let a = row(1);
    let mut b = a.clone();
    b.values[1] = Some((-0.0f64).to_bits());
    assert!(matches!(convert_block(&[a, b]).unwrap().entries[1].result, ConvertedRow::Error { .. }));
}

#[test]
fn mast_packed_append_matches_existing_storage_preparation() {
    let source = validate_gaia_input(StarId(0), 10, row(10).numbers()).unwrap().unwrap();
    let mut star = convert_source(source, &mut ConversionStats::default()).unwrap();
    let mut names = StarNames::default();
    star.name = Some(names.insert("label").unwrap());
    let mut old = StarStorage::default();
    let bound = old.push(star.clone()).unwrap();
    let (packed, new_bound, clipping) = StarStorage::prepare_row(star).unwrap();
    let mut reused = StarStorage::default();
    reused.push_prepared(packed, clipping);
    assert_eq!(bound.to_bits(), new_bound.to_bits());
    assert_eq!(old, reused);
}

#[test]
fn mast_diagnostic_bits_preserve_each_independent_flag_and_reject_unknown_bits() {
    for bits in 0u8..32 {
        let stats = ConversionStats {
            missing_motion: u64::from(bits & 1 != 0), angular_only: u64::from(bits & 2 != 0),
            missing_rv: u64::from(bits & 4 != 0), missing_color: u64::from(bits & 8 != 0), saturated_color: u64::from(bits & 16 != 0),
        };
        assert_eq!(pack_stats(&stats), bits);
    }
    let mut data = convert_block(&[row(10)]).unwrap();
    if let ConvertedRow::Ready { stats, .. } = &mut data.entries[0].result { *stats = 0x80; } else { panic!("expected prepared row"); }
    assert!(validate_block(&data, 1).is_err());
}

#[test]
#[ignore = "local sample microbenchmark; requires ASTROTERM_MAST_SAMPLE, no network"]
fn mast_measure_compact_preparation_and_labels() {
    use std::{time::Instant, hint::black_box};
    let path = std::env::var_os("ASTROTERM_MAST_SAMPLE").expect("set a normalized 13-column Parquet sample");
    let rows = read_mast_block(std::fs::read(path).unwrap()).unwrap();
    let mut stats = ConversionStats::default();
    let mut stars = Vec::new();
    let mut labels = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if let Some(source) = validate_gaia_input(StarId(i as u32), row.source_id, row.numbers()).unwrap() {
            stars.push(convert_source(source, &mut stats).unwrap());
            labels.push(format!("Gaia DR3 {}", row.source_id));
        }
    }
    let started = Instant::now();
    for star in &stars { black_box(StarStorage::prepare_row(star.clone()).unwrap()); }
    let prepare = started.elapsed();
    let started = Instant::now();
    let mut names = StarNames::default();
    for label in &labels { black_box(names.insert(label).unwrap()); }
    eprintln!("usable={}; compact preparation={prepare:?}; label insertion={:?}", stars.len(), started.elapsed());
}

#[derive(Default)]
struct LoadingProgress {
    text: Vec<u8>,
    stages: Vec<crate::progress::Stage>,
    active: Option<crate::progress::Stage>,
    totals: Vec<u64>,
    positions: Vec<u64>,
    completed: usize,
    activity_bytes: Vec<(u64, u64)>,
    finished: Vec<(crate::progress::Stage, crate::progress::Outcome)>,
}
impl std::io::Write for LoadingProgress {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.text.extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl DatasetProgress for LoadingProgress {
    fn begin_stage(&mut self, stage: crate::progress::Stage, _: &str, unit: crate::progress::Unit, total: Option<u64>) {
        self.stages.push(stage);
        self.active = Some(stage);
        if stage == crate::progress::Stage::Convert {
            assert_eq!(unit, crate::progress::Unit::Blocks);
            self.totals.push(total.unwrap());
        }
    }
    fn set_position(&mut self, position: u64) {
        if self.active == Some(crate::progress::Stage::Convert) { self.positions.push(position); }
    }
    fn set_read_progress(&mut self, bytes: u64, total: u64) {
        assert!(bytes <= total);
        self.activity_bytes.push((bytes, total));
    }
    fn finish_stage(&mut self, outcome: crate::progress::Outcome) {
        if let Some(stage) = self.active { self.finished.push((stage, outcome)); }
        if self.active == Some(crate::progress::Stage::Convert) && outcome == crate::progress::Outcome::Complete { self.completed += 1; }
        self.active = None;
    }
}

/// Small real Parquet input lets cold and cached loading exercise the same progress contract without downloads.
fn write_raw_block(rows: &[MastRow]) -> Vec<u8> {
    use parquet::{data_type::{Int64Type, DoubleType}, file::writer::SerializedFileWriter, schema::parser::parse_message_type};
    use std::sync::Arc;
    let schema = parse_message_type("message gaia {
        REQUIRED INT64 source_id;
        REQUIRED DOUBLE ra; REQUIRED DOUBLE dec; REQUIRED DOUBLE ref_epoch;
        REQUIRED DOUBLE pmra; REQUIRED DOUBLE pmdec; REQUIRED DOUBLE parallax;
        REQUIRED DOUBLE parallax_over_error; REQUIRED DOUBLE radial_velocity;
        REQUIRED DOUBLE phot_g_mean_mag; REQUIRED DOUBLE phot_bp_mean_mag; REQUIRED DOUBLE phot_rp_mean_mag;
    }").unwrap();
    let mut writer = SerializedFileWriter::new(Vec::new(), Arc::new(schema), Default::default()).unwrap();
    let mut group = writer.next_row_group().unwrap();
    for index in 0..12 {
        let mut column = group.next_column().unwrap().unwrap();
        match index {
            0 => { column.typed::<Int64Type>().write_batch(&rows.iter().map(|row| row.source_id as i64).collect::<Vec<_>>(), None, None).unwrap(); },
            _ => { column.typed::<DoubleType>().write_batch(&rows.iter().map(|row| f64::from_bits(row.values[index - 1].unwrap())).collect::<Vec<_>>(), None, None).unwrap(); },
        }
        column.close().unwrap();
    }
    group.close().unwrap();
    writer.into_inner().unwrap()
}

#[test]
fn mast_loading_uses_one_bar_for_cold_cached_and_mixed_blocks() {
    let groups = [vec![row(10), row(20)], vec![row(20), row(30), row(40)]];
    let mut expected = None;
    for cached in [[false, false], [true, false], [false, true], [true, true]] {
        let directory = tempfile::tempdir().unwrap();
        let (mut selected, _, policy) = selection(directory.path(), &groups, 4);
        let store = BlobStore::new(directory.path().join("loading-cache"));
        for ((block, rows), cached) in selected.blocks.iter_mut().zip(&groups).zip(cached) {
            let blob = selected.store.publish(&block.key, &write_raw_block(rows), false).unwrap();
            block.hash = blob.hash;
            block.stamp = blob.stamp;
            let key = block_key(block, &policy);
            if cached { store.publish(&key, &encode_mast(&convert_block(rows).unwrap()).unwrap(), false).unwrap(); }
        }
        let mut progress = LoadingProgress::default();
        let mut times = StepTimes::with_trace(true);
        let catalog = super::super::assembly::assemble_selected_catalog(&mut selected, &store, &policy, &mut progress, &mut times).unwrap();
        assert_eq!(progress.totals, vec![2]);
        assert_eq!(progress.positions, vec![0, 1, 2]);                                           // a partial final block still counts once, including skipped duplicate rows
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.stages, [crate::progress::Stage::Convert, crate::progress::Stage::CheckIds, crate::progress::Stage::AssignRegions, crate::progress::Stage::Sort, crate::progress::Stage::Reorder, crate::progress::Stage::Index]);
        let conversions = times.trace().unwrap().steps.iter().filter(|step| step.name == "MAST block conversion").count();
        assert_eq!(conversions, cached.iter().filter(|&&hit| !hit).count());
        if let Some(expected) = &expected { assert_eq!(&catalog, expected); } else { expected = Some(catalog); }
    }
}

#[test]
fn mast_loading_does_not_count_a_block_that_fails_during_assembly() {
    let directory = tempfile::tempdir().unwrap();
    let mut invalid = row(30);
    invalid.values[0] = Some(f64::NAN.to_bits());
    let groups = [vec![row(10), row(20)], vec![invalid]];
    let (mut selected, store, policy) = selection(directory.path(), &groups, 3);
    let mut progress = LoadingProgress::default();
    assert!(super::super::assembly::assemble_selected_catalog(&mut selected, &store, &policy, &mut progress, &mut StepTimes::default()).is_err());
    assert_eq!(progress.totals, vec![2]);
    assert_eq!(progress.positions, vec![0, 1]);
    assert_eq!(progress.completed, 0);
}

#[test]
fn gaia_source_ids_survive_sort_cache_and_label_consumers_without_name_storage() {
    use crate::{sky::{write_cached_catalog, load_cached_catalog, catalog_fingerprint}, model::{ObservedStar, ObservedStarView}};
    let root = tempfile::tempdir().unwrap();
    let mut groups = vec![vec![row((1 << 55) + 7), row((1 << 55) + 99), row((1 << 55) + 31)]];
    groups[0][0].values[0] = Some(270f64.to_bits());
    groups[0][1].values[8] = Some(2f64.to_bits());
    let (mut selected, store, policy) = selection(root.path(), &groups, 3);
    let prepared = super::super::assembly::assemble_selected_catalog(&mut selected, &store, &policy, &mut NoProgress(io::sink()), &mut StepTimes::default()).unwrap();
    let path = root.path().join("prepared");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    let cached = load_cached_catalog(&path, &fingerprint).unwrap();
    assert_eq!(prepared, cached);
    let catalog = &cached.catalog;
    assert!(catalog.stars.name_entries().is_empty());
    assert!(catalog.names.bytes().is_empty() && catalog.names.boundaries().is_empty() && catalog.names.ascii_alternatives().is_empty());
    let mut label = String::with_capacity(64);
    let buffer = label.as_ptr();
    for (index, id) in catalog.stars.source_ids().unwrap().iter().enumerate() {
        assert_eq!(*id, groups[0][catalog.stars.id(index).0 as usize].source_id);
        assert!(catalog.stars.has_label(index));
        for unicode in [false, true] {
            catalog.stars.write_label(index, &catalog.names, unicode, &mut label);
            assert_eq!(label, format!("Gaia DR3 {id}"));
            assert_eq!(label.as_ptr(), buffer);
            let state = ObservedStar { source_index: index, drawable: true, magnitude: 1.0, position: Default::default() };
            let view = ObservedStarView { state: crate::model::ObservedStarState::Owned(state), catalog: &catalog.stars };
            assert_eq!(crate::scene::format_star_label(&view, &catalog.names, unicode), label);
        }
    }
    let table: &dyn crate::state::Table = &catalog.stars;
    assert_eq!(table.columns()[6].name, "gaia_source_id");
    assert_eq!(table.preview().len(), 3);
}

#[test]
fn cached_block_activity_and_failed_id_check_do_not_claim_preparation_success() {
    use crate::progress::{Stage, Outcome};
    let root = tempfile::tempdir().unwrap();
    let groups = vec![vec![row(1), row(2), row(1)]];
    let (mut selection, store, policy) = selection(root.path(), &groups, 3);
    let mut progress = LoadingProgress::default();
    let mut times = StepTimes::with_trace(true);
    let error = super::super::assembly::assemble_selected_catalog(&mut selection, &store, &policy, &mut progress, &mut times).unwrap_err();
    assert!(error.to_string().contains("nonadjacent"));
    assert_eq!(progress.positions, [0, 1]);                   // byte reads never masquerade as completed blocks
    assert!(progress.activity_bytes.iter().any(|&(bytes, total)| bytes > 0 && bytes == total));
    assert_eq!(progress.active, Some(Stage::CheckIds));
    assert!(!progress.finished.contains(&(Stage::CheckIds, Outcome::Complete)));
    assert!(!progress.stages.contains(&Stage::AssignRegions));
    for name in ["MAST converted block read", "MAST converted block decode", "MAST assembly capacity", "MAST row and ID assembly", "Gaia source-ID sort", "Gaia source-ID uniqueness"] {
        assert!(times.trace().unwrap().steps.iter().any(|step| step.name == name), "missing {name}");
    }
}

#[test]
fn prepared_hit_verifies_its_own_data_without_opening_damaged_raw_payloads() {
    let root = tempfile::tempdir().unwrap();
    let (mut selected, converted, policy) = selection(root.path(), &[vec![row(10), row(20)]], 2);
    let expected = super::super::assembly::assemble_selected_catalog(&mut selected, &converted, &policy,
        &mut NoProgress(io::sink()), &mut StepTimes::default()).unwrap();
    let (path, fingerprint) = super::super::selection_cache_identity(&selected, &policy, root.path()).unwrap();
    super::super::save_prepared_catalog(path, &expected, &fingerprint, &mut NoProgress(io::sink()), &mut StepTimes::default()).unwrap();
    let block = &selected.blocks[0];
    let raw_path = root.path().join("raw/objects").join(&block.hash);
    let modified = std::fs::metadata(&raw_path).unwrap().modified().unwrap();
    std::fs::write(&raw_path, [99]).unwrap();
    std::fs::File::options().write(true).open(&raw_path).unwrap().set_modified(modified).unwrap();
    let reads = selected.store.payload_reads(&block.hash);
    let mut times = StepTimes::with_trace(true);
    let actual = super::super::prepare_selected_catalog(&mut selected, root.path(), &mut NoProgress(io::sink()), &mut times).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(selected.store.payload_reads(&selected.blocks[0].hash), reads);
    assert!(!times.trace().unwrap().steps.iter().any(|step| step.name == "MAST block conversion"));
}

#[test]
fn repaired_hash_controls_converted_and_prepared_keys_without_reconverting_earlier_blocks() {
    let root = tempfile::tempdir().unwrap();
    let groups = [vec![row(10)], vec![row(20)]];
    let (mut selected, original, policy) = selection(root.path(), &groups, 2);
    let converted = BlobStore::new(root.path().join("new-converted"));
    let first_key = block_key(&selected.blocks[0], &policy);
    converted.publish(&first_key, &original.read(&first_key).unwrap().unwrap(), false).unwrap();
    let old_second_key = block_key(&selected.blocks[1], &policy);
    let old_identity = super::super::selection_cache_identity(&selected, &policy, root.path()).unwrap();
    std::fs::write(old_identity.0.as_ref().unwrap(), b"old invalid prepared file").unwrap();
    let reads = selected.store.payload_reads(&selected.blocks[0].hash);
    let mut times = StepTimes::with_trace(true);
    let mut progress = LoadingProgress::default();
    progress.begin_stage(crate::progress::Stage::Convert, "Loading MAST blocks", crate::progress::Unit::Blocks, Some(2));
    let first = load_with_raw_reader(&selected.store, &mut selected.blocks[0], &converted, &policy, &mut progress, &mut times,
        |_, _, _, _| panic!("the earlier converted block must not read raw input")).unwrap();
    assert_eq!(first.entries[0].id, 10);
    progress.set_position(1);
    load_with_raw_reader(&selected.store, &mut selected.blocks[1], &converted, &policy, &mut progress, &mut times,
        |store, block, progress, _| {
            progress.set_detail("Fixture: pinned repair produced different writer bytes");
            let bytes = write_raw_block(&groups[1]);
            let repaired = store.publish(&block.key, &bytes, false)?;
            block.hash = repaired.hash;
            block.stamp = repaired.stamp;
            Ok(bytes)
        }).unwrap();
    assert_eq!(progress.positions, [1]);                     // repairing a block does not restart or advance the loading bar
    let new_second_key = block_key(&selected.blocks[1], &policy);
    assert_ne!(old_second_key, new_second_key);
    assert!(converted.read(&old_second_key).unwrap().is_none());
    assert!(converted.read(&new_second_key).unwrap().is_some());
    assert_eq!(selected.store.payload_reads(&selected.blocks[0].hash), reads);
    assert_eq!(times.trace().unwrap().steps.iter().filter(|step| step.name == "MAST block conversion").count(), 1);
    let new_identity = super::super::selection_cache_identity(&selected, &policy, root.path()).unwrap();
    assert_ne!(old_identity, new_identity);
    selected.check_unchanged().unwrap();
    let prepared = super::super::assembly::assemble_selected_catalog(&mut selected, &converted, &policy, &mut NoProgress(io::sink()), &mut StepTimes::default()).unwrap();
    super::super::save_prepared_catalog(new_identity.0.clone(), &prepared, &new_identity.1, &mut NoProgress(io::sink()), &mut StepTimes::default()).unwrap();
    assert_eq!(crate::sky::load_cached_catalog(new_identity.0.as_ref().unwrap(), &new_identity.1).unwrap(), prepared);
    let located = selected.store.locate(&selected.blocks[1].key).unwrap().unwrap();
    selected.blocks[1].hash = located.hash;
    selected.blocks[1].stamp = located.stamp;
    assert_eq!(super::super::selection_cache_identity(&selected, &policy, root.path()).unwrap(), new_identity);
    assert_eq!(std::fs::read(old_identity.0.as_ref().unwrap()).unwrap(), b"old invalid prepared file");
}

#[test]
fn scientific_row_errors_are_not_raw_repair_requests() {
    let root = tempfile::tempdir().unwrap();
    let mut input = row(10);
    input.values[0] = Some(f64::NAN.to_bits());
    let (mut selected, _, policy) = selection(root.path(), &[vec![input.clone()]], 1);
    let blob = selected.store.publish(&selected.blocks[0].key, &write_raw_block(&[input]), false).unwrap();
    selected.blocks[0].hash = blob.hash;
    selected.blocks[0].stamp = blob.stamp;
    let converted = BlobStore::new(root.path().join("fresh-converted"));
    let mut progress = NoProgress(Vec::new());
    let data = load_converted_block(&selected.store, &mut selected.blocks[0], &converted, &policy, &mut progress, &mut StepTimes::default()).unwrap();
    assert!(matches!(data.entries[0].result, ConvertedRow::Error { .. }));
    assert!(!String::from_utf8(progress.0).unwrap().contains("Repairing"));
}

#[test]
fn planned_capacity_survives_heavy_skips_duplicates_and_an_empty_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut missing = row(20);
    missing.values[8] = None;
    let groups = vec![vec![row(10), row(10), missing.clone(), missing.clone()],
        vec![missing.clone(), missing, row(30), row(30), row(30), row(40)]];
    for count in [0, 1, 4, 7, 10] {
        let (mut selected, store, policy) = selection(root.path(), &groups, count);
        let mut times = StepTimes::with_trace(true);
        let prepared = super::super::assembly::assemble_selected_catalog(&mut selected, &store, &policy, &mut NoProgress(io::sink()), &mut times).unwrap();
        let mut expected = Vec::new();
        for (index, original) in groups.iter().flatten().take(count).enumerate() {
            if original.source_id == 20 || expected.last().is_some_and(|&(_, id)| id == original.source_id) { continue; }
            expected.push((index as u32, original.source_id));
        }
        let stars = &prepared.catalog.stars;
        let mut actual: Vec<_> = stars.columns().id.iter().copied().zip(stars.source_ids().unwrap().iter().copied()).collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        let c = stars.owned_columns();
        for capacity in [c.u0.capacity(), c.w.capacity(), c.magnitude.capacity(), c.brightness_key.capacity(),
            c.distance.capacity(), c.id.capacity(), c.display_color.capacity(), stars.owned_source_ids().unwrap().capacity(),
            prepared.preparation.motion_bounds.capacity()] {
            assert_eq!(capacity, count);                                                        // skipped rows remain spare capacity through final sorting and preparation
        }
        assert_eq!(stars.label_capacity(), count * 8);
        assert!(stars.name_entries().is_empty());
        assert!(prepared.catalog.names.bytes().is_empty() && prepared.catalog.names.boundaries().is_empty());
        assert_eq!(times.trace().unwrap().steps.iter().filter(|step| step.name == "MAST assembly capacity").count(), 1);
    }
}
