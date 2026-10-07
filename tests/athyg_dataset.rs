//! Checks against the real AT-HYG file in `datasets/`, which isn't part of the repository. Run with
//! `cargo test --release -- --ignored` after downloading it (see the README).

use std::path::Path;

use astroterm::catalog::{Catalog, CatalogStar, load_athyg_catalog, load_embedded_catalog};
use astroterm::model::SkyCatalog;

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
    let sky = astroterm::sky::prepare_catalog(&athyg).unwrap();
    assert_eq!(sky.catalog.constellations().len(), 88);
    assert_eq!(sky.catalog.stars.len(), athyg.stars.iter().filter(|star| star.has_data).count());
    assert_eq!(sky.catalog.grid.offsets[astroterm::model::SIMULATION_REGION_COUNT], sky.catalog.stars.len());
    for (cell, range) in sky.catalog.grid.offsets.windows(2).take(astroterm::model::CELL_COUNT).enumerate() {
        for index in range[0]..range[1] {
            assert_eq!(astroterm::model::hash_direction(astroterm::model::GRID_DEPTH, sky.catalog.stars.stored_direction(index)), cell);
        }
    }
    let constellation_start = sky.catalog.grid.offsets[astroterm::model::CONSTELLATION_REGION];
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
    let source = load_athyg_catalog(Path::new(DATASET)).unwrap();
    let stored = astroterm::sky::prepare_catalog(&source).unwrap();
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

#[test]
#[ignore = "needs datasets/athyg_40.csv.gz; release cache roundtrip"]
fn real_catalog_cache_preserves_the_rendered_frame() {
    use astroterm::astro::{J2000, Observer};
    use astroterm::canvas::Canvas;
    use astroterm::model::{Sky, ProjectionViewport as Viewport, View, RenderOptions};
    use astroterm::projection::project_sky;
    use astroterm::scene::draw_sky_scene;
    use astroterm::sky::{update_sky_positions, catalog_fingerprint, load_cached_catalog, write_cached_catalog};
    use astroterm::timing::StepTimes;
    fn frame(catalog: SkyCatalog) -> Canvas {
        let mut sky = Sky::new(std::sync::Arc::new(catalog));
        update_sky_positions(&mut sky, J2000, &Observer::default(), 5.0, &mut StepTimes::default());
        let mut canvas = Canvas::new(41, 81);
        let options = RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            constellations: true,
            grid: false,
            magnitude_threshold: 5.0,
            label_threshold: 0.25,
            dynamic_names: true,
        };
        draw_sky_scene(
            &mut canvas,
            &options,
            &project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 }).view(&sky),
        );
        canvas
    }
    let catalog = astroterm::sky::prepare_owned_catalog(load_athyg_catalog(Path::new(DATASET)).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &catalog, &fingerprint).unwrap();
    let expected = frame(catalog.catalog);
    let cached = load_cached_catalog(&path, &fingerprint).unwrap();
    assert!(cached.catalog.stars.len() > 2_500_000);
    assert_eq!(frame(cached.catalog), expected);
}
