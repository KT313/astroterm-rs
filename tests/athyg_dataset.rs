//! Checks against the real AT-HYG file in `datasets/`, which isn't part of the repository. Run with
//! `cargo test --release -- --ignored` after downloading it (see the README).

use std::path::Path;

use astroterm::catalog::{Catalog, CatalogStar, load_athyg_catalog, load_embedded_catalog};

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
    let athyg = load_athyg_catalog(Path::new(DATASET), &mut astroterm::progress::Report::quiet()).expect("dataset loads");
    let embedded = load_embedded_catalog(&mut astroterm::progress::Report::quiet()).expect("embedded catalog loads");
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
    let sky = astroterm::sky::prepare_catalog(&athyg, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(sky.catalog.constellations().len(), 88);
    assert_eq!(sky.catalog.stars.len(), athyg.stars.iter().filter(|star| star.has_data).count());
    assert_eq!(sky.catalog.grid.offsets[astroterm::constants::SIMULATION_REGION_COUNT], sky.catalog.stars.len());
    for (cell, range) in sky.catalog.grid.offsets.windows(2).take(astroterm::constants::CELL_COUNT).enumerate() {
        for index in range[0]..range[1] {
            assert_eq!(astroterm::model::hash_direction(astroterm::constants::GRID_DEPTH, sky.catalog.stars.stored_direction(index)), cell);
        }
    }
    let constellation_start = sky.catalog.grid.offsets[astroterm::constants::CONSTELLATION_REGION];
    assert_eq!(sky.catalog.stars.len() - constellation_start, 692);
    assert!(sky.catalog.endpoint_indices().iter().copied().eq(constellation_start..sky.catalog.stars.len()));
    eprintln!("All {} AT-HYG stars partition into ordinary regions plus 692 exclusive endpoints", sky.catalog.stars.len());
    assert_eq!(
        sky.catalog.constellations()
            .iter()
            .map(|figure| figure.segments.len())
            .sum::<usize>(),
        676
    );
}

#[test]
#[ignore = "needs datasets/athyg_40.csv.gz; release quantization audit"]
fn real_catalog_quantization_stays_within_half_an_arcsecond() {
    let source = load_athyg_catalog(Path::new(DATASET), &mut astroterm::progress::Report::quiet()).unwrap();
    let stored = astroterm::sky::prepare_catalog(&source, &mut astroterm::progress::Report::quiet()).unwrap();
    let (start, end) = astroterm::astro::models::stars::computational_years();
    let mut maximum = 0.0_f64;
    for star in stored.catalog.stars.iter() {
        let i = source.stars.binary_search_by_key(&star.id, |s| s.id).unwrap();
        let mut original = astroterm::sky::prepare_star(&source.stars[i]).motion;
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
        stored.catalog.stars.precise_count()
    );
}

/// The rebuilt pipeline's group tables from the real AT-HYG catalog: built from the source, validated, and equal
/// to the tables built from the prepared cache written on the way.
#[test]
#[ignore = "needs datasets/athyg_40.csv.gz"]
fn athyg_builds_validated_group_tables_from_source_and_cache() {
    use astroterm::catalog::datasets::{Dataset, DatasetDirectories};
    use astroterm::progress::NoProgress;
    use astroterm::timing::StepTimes;
    let cache = tempfile::tempdir().unwrap();
    let directories = DatasetDirectories { data: None, cache: Some(cache.path().into()) };
    let dataset = Dataset::Path(DATASET.into());
    let mut notices = Vec::new();
    let started = std::time::Instant::now();
    let prepared = astroterm::sky::load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(&mut notices), &mut astroterm::timing::StepTimes::default())).unwrap();
    eprintln!("prepared {} stars in {:?}", prepared.catalog.stars.len(), started.elapsed());
    let started = std::time::Instant::now();
    let mut times = StepTimes::with_trace(true);
    let tables = astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::live(&mut NoProgress(std::io::sink()), &mut times)).unwrap();
    eprintln!("built and validated the group tables in {:?}", started.elapsed());
    for step in &times.trace().unwrap().steps { eprintln!("  {} {:.3}s {}", step.name, step.seconds, step.details.join("; ")); }
    assert_eq!(tables.region_star_count() + tables.constellation_stars.len(), prepared.catalog.stars.len());
    assert!(tables.region_star_count() > 2_500_000);
    assert_eq!(tables.constellation_stars.len(), prepared.catalog.endpoint_indices().len());
    let cached = astroterm::sky::load_sky_catalog(Some(&dataset), astroterm::constants::MAST_PARALLEL_BLOCKS, &directories, &mut astroterm::progress::Report::live(&mut astroterm::progress::NoProgress(&mut notices), &mut astroterm::timing::StepTimes::default())).unwrap();
    assert_eq!(cached, prepared);
    let from_cache = astroterm::sky::build_sky_tables(&cached, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(from_cache, tables);
}
