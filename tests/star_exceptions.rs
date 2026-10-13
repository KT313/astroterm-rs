//! Stage-3 skeleton rejects unsupported input explicitly; accepted catalogs keep empty exception storage.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, constants::MAST_PARALLEL_BLOCKS, model::StarException,
    progress::NoProgress, sky::{load_sky_catalog, prepare_owned_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint},
    timing::StepTimes};
use std::io;

#[test]
fn ordinary_catalog_exposes_an_empty_allocation_free_exception_table() {
    let prepared = prepare_owned_catalog(astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(prepared.catalog.star_exceptions.capacity(), 0);
    assert_eq!(prepared.catalog.stars.precise_count(), 0);
    prepared.catalog.validate_exception_support().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("catalog");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    let loaded = load_cached_catalog(&path, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(loaded, prepared);
    assert_eq!(loaded.catalog.star_exceptions.capacity(), 0);
    astroterm::sky::build_sky_tables(&loaded, &mut astroterm::progress::Report::quiet()).unwrap(); // the frame loop's tables accept it
}

#[test]
fn supported_fast_motion_prepares_without_exceptions_and_is_filed_in_its_region() {
    use astroterm::astro::{J2000, JULIAN_YEAR_DAYS};
    use astroterm::constants::GRID_DEPTH;
    use astroterm::model::hash_direction;
    use astroterm::regions::rows_wanted;
    use astroterm::model::speed_group;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fast.csv");
    std::fs::write(&path, "ra,dec,mag,pmdec\n1,60,4,20626\n").unwrap();                        // 0.0001 rad/yr in declination, no distance
    let prepared = prepare_owned_catalog(astroterm::catalog::load_athyg_catalog(&path, &mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
    assert!(prepared.catalog.star_exceptions.is_empty());
    assert_eq!(prepared.catalog.singular_count, 0);
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap();
    let (region, group, table) = tables.star_tables().find(|(_, _, table)| !table.is_empty()).unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(region, hash_direction(GRID_DEPTH, table.stored_direction(0)));                 // filed by its catalog-epoch direction
    assert_eq!(group, speed_group(table.motion_bound(0)));
    let sample = table.motion(0).evaluate(1000.0, 4.0);
    assert!(sample.direction.x.is_finite() && sample.direction.y.is_finite() && sample.direction.z.is_finite());
    assert_eq!(rows_wanted(table, J2000 + 1000.0 * JULIAN_YEAR_DAYS, 5.0), 1);
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
        let error = load_sky_catalog(Some(&Dataset::Path(path)), MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut NoProgress(io::sink()), &mut StepTimes::default())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains(reason), "{error}");
        assert!(error.to_string().contains("Star 0"));
        assert!(!cache.exists() || std::fs::read_dir(cache).unwrap().next().is_none());
    }
}
