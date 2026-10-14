//! Opt-in qualification of up to two real Gaia files; normal tests never download catalogs.
use astroterm::{catalog::datasets::{Dataset, DatasetDirectories}, sky::load_sky_catalog};

#[test]
#[ignore = "requires ASTROTERM_GAIA_DATA and explicitly opted-in Gaia files/network"]
fn real_gaia_prefix_converts_and_reuses_prepared_cache() {
    let data = std::env::var_os("ASTROTERM_GAIA_DATA").expect("set the astroterm data directory");
    let count: u16 = std::env::var("ASTROTERM_GAIA_FILES").unwrap_or("1".into()).parse().unwrap();
    assert!((1..=2).contains(&count), "qualification is limited to two real files");
    let cache = tempfile::tempdir().unwrap();
    let dirs = DatasetDirectories { data: Some(data.into()), cache: Some(cache.path().into()) };
    let dataset = Dataset::GaiaGdr3 { files: count };
    let mut notices = Vec::new();
    let original = load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(&mut notices), &mut astroterm::time::StepTimes::default())).unwrap();
    println!("{}", String::from_utf8_lossy(&notices));
    assert!(original.catalog.stars.len() > 100_000);
    assert!(original.catalog.constellations().is_empty());
    notices.clear();
    let cached = load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(&mut notices), &mut astroterm::time::StepTimes::default())).unwrap();
    assert_eq!(cached, original);
    assert!(!String::from_utf8_lossy(&notices).contains("Gaia DR3: rows="));
    assert_eq!(String::from_utf8_lossy(&notices).matches("no G-to-V conversion").count(), 1);
    println!("Prepared cache equality passed: {} stars from {count} files", original.catalog.stars.len());
}

/// The rebuilt pipeline's group tables from Gaia rows: Gaia labels are source IDs and there is no constellation
/// table. Uses the first MAST row group only, converted from the local parquet block into a temporary cache.
#[test]
#[ignore = "requires ASTROTERM_GAIA_DATA with the first MAST parquet block; at most the first row group"]
fn mast_first_block_builds_validated_group_tables() {
    use astroterm::model::StarLabels;
    let data = std::env::var_os("ASTROTERM_GAIA_DATA").expect("set the astroterm data directory");
    let cache = tempfile::tempdir().unwrap();
    let dirs = DatasetDirectories { data: Some(data.into()), cache: Some(cache.path().into()) };
    let dataset = Dataset::GaiaGdr3Stars { stars: 100_000 };
    let mut notices = Vec::new();
    let prepared = load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &dirs, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(&mut notices), &mut astroterm::time::StepTimes::default())).unwrap();
    println!("{}", String::from_utf8_lossy(&notices));
    assert!(prepared.catalog.stars.source_ids().is_some());
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(tables.region_star_count(), prepared.catalog.stars.len());
    assert!(tables.constellation_stars.is_empty());
    assert!(tables.star_tables().all(|(_, _, table)| matches!(table.labels(), StarLabels::SourceIds(_))));
    let labelled: usize = tables.star_tables().map(|(_, _, table)| table.labels().len()).sum();
    assert_eq!(labelled, tables.region_star_count());
    println!("Gaia group tables validated: {} stars", tables.region_star_count());
}
