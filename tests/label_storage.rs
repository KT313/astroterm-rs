//! Shared labels, exact ASCII/Unicode alternatives, unified eligibility and stable compact IDs.
use astroterm::catalog::{load_athyg_catalog, StarId};
use astroterm::sky::{prepare_catalog, create_sky_from_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint};
use astroterm::model::{RenderOptions, View, ProjectionViewport};
use astroterm::astro::Vector3;
use astroterm::canvas::Canvas;

fn source() -> astroterm::catalog::Catalog {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("labels.csv");
    std::fs::write(&path, concat!(
        "ra,dec,mag,proper,bayer,con,hip,gaia\n",
        "0,0,-1,Named 星,,,,\n",
        "1,0,-1,,Alp-2,Cen,,\n",
        "2,0,-1,,,,42,\n",
        "3,0,-1,,,,42,\n",
        "4,0,-1,,,,,18446744073709551615\n",
        "5,0,-1,,,,,\n",
        ",0,2,,,,,\n",
        "6,0,-1,Named 星,,,,\n",
    )).unwrap();
    load_athyg_catalog(&path).unwrap()
}

#[test]
fn labels_are_shared_and_cached_without_changing_source_ids() {
    let source = source();
    assert_eq!(source.stars.last().unwrap().id, StarId(7)); // skipped source row still occupies an ID
    let prepared = prepare_catalog(&source).unwrap();
    let catalog = &prepared.catalog;
    let name = |id| catalog.stars.iter().find(|star| star.id == StarId(id)).unwrap().name;
    assert_eq!(name(0), name(7));
    assert_eq!(name(2), name(3));
    assert_eq!(catalog.names.get(name(5)), None);
    assert_eq!(catalog.names.get_for_mode(name(1), true), Some("α² Cen"));
    assert_eq!(catalog.names.get_for_mode(name(1), false), Some("Alp2 Cen"));
    assert_eq!(catalog.names.get(name(4)), Some("Gaia 18446744073709551615"));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prepared");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    let restored = load_cached_catalog(&path, &fingerprint).unwrap();
    assert_eq!(restored, prepared);
    assert_eq!(restored.catalog.names.get_for_mode(name(1), false), Some("Alp2 Cen"));
}

#[test]
fn identifiers_and_proper_names_follow_the_same_brightest_star_rule() {
    let source = source();
    for (id, unicode, expected) in [(0, false, "Named 星"), (1, false, "Alp2 Cen"), (1, true, "α² Cen"), (2, false, "HIP 42")] {
        let mut sky = create_sky_from_catalog(&source).unwrap();
        for star in &mut sky.stars {
            star.position = Vector3 { x: 0.0, y: 0.0, z: -1.0 };
        }
        for planet in &mut sky.planets { planet.position = Vector3 { x: 0.0, y: 0.0, z: -1.0 }; }
        sky.moon.position = Vector3 { x: 0.0, y: 0.0, z: -1.0 };
        let index = sky.star_views().position(|star| star.id() == StarId(id)).unwrap();
        sky.stars[index].position = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let options = RenderOptions { unicode, braille: false, color: false, constellations: false, grid: false,
            magnitude_threshold: 5.0, dynamic_names: true };
        let view = View::default();
        let projected = astroterm::projection::project_sky(&sky, &view, ProjectionViewport { width: 80, height: 40 });
        let mut canvas = Canvas::new(40, 80);
        astroterm::scene::draw_sky_scene(&mut canvas, &options, &projected.view(&sky));
        assert!(canvas.to_lines().join("\n").contains(expected), "missing {expected}");
    }
}

#[test]
fn ids_reject_overflow_and_preserve_the_maximum_without_renumbering() {
    assert_eq!(StarId::try_from_index(u64::from(u32::MAX)).unwrap(), StarId(u32::MAX));
    assert!(StarId::try_from_index(u64::from(u32::MAX) + 1).is_err());
    let mut source = source();
    source.stars.truncate(1);
    source.stars[0].id = StarId(u32::MAX);
    let prepared = prepare_catalog(&source).unwrap();
    assert_eq!(prepared.catalog.stars.id(0), StarId(u32::MAX));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prepared");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    assert_eq!(load_cached_catalog(&path, &fingerprint).unwrap().catalog.stars.id(0), StarId(u32::MAX));
}
