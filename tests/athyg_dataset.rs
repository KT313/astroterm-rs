//! Checks against the real AT-HYG file in `datasets/`, which isn't part of the repository. Run with
//! `cargo test --release -- --ignored` after downloading it (see the README).

use std::path::Path;

use astroterm::catalog::{Catalog, CatalogStar, load_athyg_catalog, load_embedded_catalog};
use astroterm::sky::{SkyCatalog, Star};

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
        let find = |catalog: &Catalog| {
            catalog
                .stars
                .iter()
                .find(|star| catalog.names.get(star.name) == Some(name))
                .cloned()
        };
        let (from_athyg, from_embedded) = (find(&athyg).expect(name), find(&embedded).expect(name));
        let separation = separation_degrees(&from_athyg, &from_embedded);
        assert!(separation < 0.05, "{name} is {separation}° off");
        let magnitude_difference = (from_athyg.magnitude - from_embedded.magnitude).abs();
        assert!(magnitude_difference < 0.05, "{name} magnitude");
    }

    // nearly all constellation figures find their stars
    let sky = SkyCatalog::from_catalog(&athyg);
    assert_eq!(sky.constellations.len(), 88);
    assert_eq!(
        sky.constellations
            .iter()
            .map(|figure| figure.segments.len())
            .sum::<usize>(),
        676
    );
}

#[test]
#[ignore = "needs datasets/athyg_40.csv.gz; release quantization audit"]
fn real_catalog_quantization_stays_within_half_an_arcsecond() {
    let source = load_athyg_catalog(Path::new(DATASET)).unwrap();
    let stored = SkyCatalog::from_catalog(&source);
    let (start, end) = astroterm::astro::models::stars::computational_years();
    let mut maximum = 0.0_f64;
    for star in stored.stars.iter() {
        let i = source.stars.binary_search_by_key(&star.id, |s| s.id).unwrap();
        let mut original = Star::from_catalog_star(&source.stars[i]).motion;
        original.remove_singular_distance();
        for t in [start, end, original.closest_approach(start, end).0] {
            let a = original.evaluate(t, star.magnitude).direction;
            let b = star.motion.evaluate(t, star.magnitude).direction;
            let error = a.cross(b).length().atan2(a.dot(b)).to_degrees() * 3600.0;
            maximum = maximum.max(error);
            assert!(error <= 0.5, "{} at {t}: {error} arcsec", star.id.0);
        }
    }
    eprintln!(
        "AT-HYG max quantization separation: {maximum} arcsec; {} precise trajectories",
        stored.stars.precise_count()
    );
}
