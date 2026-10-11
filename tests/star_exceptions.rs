//! Stage-3 skeleton rejects unsupported input explicitly; accepted catalogs keep empty exception storage.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, model::StarException,
    sky::{load_sky_catalog, prepare_owned_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint},
    cli::{Arguments, build_config}, state::ApplicationState, timing::StepTimes};
use clap::Parser;
use std::io;

#[test]
fn ordinary_catalog_exposes_an_empty_allocation_free_exception_table() {
    let prepared = prepare_owned_catalog(astroterm::catalog::load_embedded_catalog().unwrap()).unwrap();
    assert_eq!(prepared.catalog.star_exceptions.capacity(), 0);
    assert_eq!(prepared.catalog.stars.precise_count(), 0);
    prepared.catalog.validate_exception_support().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("catalog");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    let loaded = load_cached_catalog(&path, &fingerprint).unwrap();
    assert_eq!(loaded, prepared);
    assert_eq!(loaded.catalog.star_exceptions.capacity(), 0);
    let config = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
    let mut state = ApplicationState::new(config, StepTimes::default());
    state.replace_catalog(loaded);
    let mut log = Vec::new();
    state.write_tables(&mut log, None).unwrap();
    let text = String::from_utf8(log).unwrap();
    let exceptions = text.split("### persistent.catalog.star\\_exceptions\n").nth(1).unwrap().split("\n### ").next().unwrap();
    assert!(exceptions.contains("**Used:** 0 B · **Reserved:** 0 B"));
    assert!(exceptions.contains("uses\\_motion\\_fallback: bool"));
    let stars = text.split("### persistent.catalog.stars\n").nth(1).unwrap().split("\n### ").next().unwrap();
    assert!(!stars.contains("data\\_flags:") && !stars.contains("precise\\_motion\\_entry:"));
}

#[test]
fn any_nonempty_exception_table_fails_the_preparation_boundary() {
    for (fallback, precise) in [(false, 0), (true, 0), (false, 1), (true, 1)] {
        let mut catalog = astroterm::model::SkyCatalog::empty();
        catalog.star_exceptions.push(StarException { catalog_row_index: 0, uses_motion_fallback: fallback, precise_motion_entry: precise });
        let error = catalog.validate_exception_support().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("not implemented"));
    }
}

#[test]
fn source_errors_are_reported_before_a_cache_is_published() {
    for (row, reason) in [("0,0,4,1,-1000", "tangential-motion fallback"), ("0,0,4,1e-50,0", "distance not representable")] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("unsupported.csv");
        std::fs::write(&path, format!("ra,dec,mag,dist,rv\n{row}\n")).unwrap();
        let cache = root.path().join("cache");
        let dirs = DatasetDirectories { data: None, cache: Some(cache.clone()) };
        let error = load_sky_catalog(Some(&Dataset::Path(path)), &dirs, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains(reason), "{error}");
        assert!(error.to_string().contains("Star 0"));
        assert!(!cache.exists() || std::fs::read_dir(cache).unwrap().next().is_none());
    }
}
