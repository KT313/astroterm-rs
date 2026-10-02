//! Checks against the real AT-HYG file in `datasets/`, which isn't part of the repository. Run with
//! `cargo test --release -- --ignored` after downloading it (see the README).

use std::path::Path;

use astroterm::catalog::{CatalogStar, load_athyg_catalog, load_embedded_catalog};
use astroterm::sky::Sky;

const DATASET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/datasets/athyg_40.csv.gz");

/// Angle between two J2000 positions, in degrees.
fn separation_degrees(a: &CatalogStar, b: &CatalogStar) -> f64 {
    let cos = a.declination.sin() * b.declination.sin()
        + a.declination.cos() * b.declination.cos() * (a.right_ascension - b.right_ascension).cos();
    cos.clamp(-1.0, 1.0).acos().to_degrees()
}

#[test]
#[ignore = "needs datasets/athyg_40.csv.gz"]
fn athyg_matches_the_embedded_catalog() {
    let athyg = load_athyg_catalog(Path::new(DATASET)).expect("dataset loads");
    let embedded = load_embedded_catalog().expect("embedded catalog loads");
    assert!(athyg.stars.len() > 2_500_000, "{} stars", athyg.stars.len());

    // bright stars are where the Yale catalog has them
    for name in ["Vega", "Sirius", "Polaris", "Arcturus", "Canopus"] {
        let find = |stars: &[CatalogStar]| stars.iter().find(|star| star.name == Some(name)).cloned();
        let (from_athyg, from_embedded) = (find(&athyg.stars).expect(name), find(&embedded.stars).expect(name));
        let separation = separation_degrees(&from_athyg, &from_embedded);
        assert!(separation < 0.05, "{name} is {separation}° off");
        let magnitude_difference = (from_athyg.magnitude - from_embedded.magnitude).abs();
        assert!(magnitude_difference < 0.5, "{name} magnitude"); // Tycho photometry is rough for the brightest stars
    }

    // nearly all constellation figures find their stars
    let sky = Sky::from_catalog(&athyg);
    assert!(sky.constellations.len() >= 85, "{} figures", sky.constellations.len());
}
