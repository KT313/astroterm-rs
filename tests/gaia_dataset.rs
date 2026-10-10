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
    let original = load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap();
    println!("{}", String::from_utf8_lossy(&notices));
    assert!(original.catalog.stars.len() > 100_000);
    assert!(original.catalog.constellations().is_empty());
    notices.clear();
    let cached = load_sky_catalog(Some(&dataset), &dirs, &mut notices).unwrap();
    assert_eq!(cached, original);
    assert!(!String::from_utf8_lossy(&notices).contains("Gaia DR3: rows="));
    assert_eq!(String::from_utf8_lossy(&notices).matches("no G-to-V conversion").count(), 1);
    println!("Prepared cache equality passed: {} stars from {count} files", original.catalog.stars.len());
}
