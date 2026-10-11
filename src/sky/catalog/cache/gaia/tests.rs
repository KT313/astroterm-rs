use super::*;
use flate2::{write::GzEncoder, Compression};
use std::{fs, io::Write, path::Path};

fn source(directory: &Path, first: u32) -> GaiaFile {
    let path = directory.join(format!("GaiaSource_{first:06}-{first:06}.csv.gz"));
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    writeln!(gzip, "# %ECSV 1.0\nsource_id,ra,dec,ref_epoch,pmra,pmdec,parallax,parallax_over_error,radial_velocity,phot_g_mean_mag,phot_bp_mean_mag,phot_rp_mean_mag").unwrap();
    writeln!(gzip, "{},45,0,2016,0,0,-1,-1,,12,13,11", (u64::from(first) << 43) + 1).unwrap();
    fs::write(&path, gzip.finish().unwrap()).unwrap();
    GaiaFile::for_test(path, first, first)
}

#[test]
fn prefix_and_source_identity_change_cache_but_extra_files_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let a = source(dir.path(), 0);
    let first = fingerprint_files(std::slice::from_ref(&a));
    let b = source(dir.path(), 1);
    assert_eq!(first, fingerprint_files(std::slice::from_ref(&a)));
    assert_ne!(first, fingerprint_files(&[a.clone(), b]));
    let mut changed = a;
    changed.entry.checksum[0] ^= 1;
    assert_ne!(first, fingerprint_files(&[changed]));
}

#[test]
fn multiple_files_prepare_owned_rows_and_hit_cache_without_reparsing() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![source(dir.path(), 0), source(dir.path(), 1)];
    let fingerprint = fingerprint_files(&files);
    let path = Some(dir.path().join("result.catalog"));
    let mut notices = Vec::new();
    let prepared = load_prepared_gaia(&files, path.clone(), &fingerprint, &mut crate::progress::NoProgress(&mut notices), &mut StepTimes::default()).unwrap();
    assert_eq!(prepared.catalog.stars.len(), 2);
    assert!(prepared.catalog.constellations().is_empty());
    let mut ids: Vec<_> = prepared.catalog.stars.iter().map(|star| star.id.0).collect();
    ids.sort_unstable(); assert_eq!(ids, [0, 1]);
    let mut label = String::new();
    for (row, id) in prepared.catalog.stars.source_ids().unwrap().iter().enumerate() {
        prepared.catalog.stars.write_label(row, &prepared.catalog.names, true, &mut label);
        assert_eq!(label, format!("Gaia DR3 {id}"));
    }
    assert!(String::from_utf8_lossy(&notices).contains("output sources=2"));
    notices.clear();
    let cached = load_prepared_gaia(&files, path.clone(), &fingerprint, &mut crate::progress::NoProgress(&mut notices), &mut StepTimes::default()).unwrap();
    assert_eq!(cached, prepared);
    assert!(!String::from_utf8_lossy(&notices).contains("output sources="));
    fs::write(path.as_ref().unwrap(), b"broken prepared cache").unwrap();
    let rebuilt = load_prepared_gaia(&files, path.clone(), &fingerprint, &mut crate::progress::NoProgress(&mut notices), &mut StepTimes::default()).unwrap();
    assert_eq!(rebuilt, prepared);
    fs::write(&files[0].path, b"corrupt").unwrap();
    assert!(load_prepared_gaia(&files, path, &fingerprint, &mut crate::progress::NoProgress(&mut notices), &mut StepTimes::default()).is_err());
}

#[test]
fn zero_catalog_and_cache_hits_always_explain_g_magnitude() {
    let dir = tempfile::tempdir().unwrap();
    let directories = DatasetDirectories { data: None, cache: Some(dir.path().into()) };
    for _ in 0..2 {
        let mut notices = Vec::new();
        let prepared = load_gaia_catalog(0, &directories, &mut crate::progress::NoProgress(&mut notices), &mut StepTimes::default()).unwrap();
        assert!(prepared.catalog.stars.is_empty());
        let text = String::from_utf8(notices).unwrap();
        assert_eq!(text.matches("no G-to-V conversion").count(), 1);
    }
}

#[test]
fn previous_label_boundary_format_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![source(dir.path(), 0)];
    let fingerprint = fingerprint_files(&files);
    let path = dir.path().join("old.catalog");
    load_prepared_gaia(&files, Some(path.clone()), &fingerprint, &mut crate::progress::NoProgress(Vec::new()), &mut StepTimes::default()).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[8..12].copy_from_slice(&7_u32.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    let error = super::super::format::load_cached_catalog(&path, &fingerprint).unwrap_err();
    assert!(error.to_string().contains("version"));
}
