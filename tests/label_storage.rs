//! Shared labels, exact ASCII/Unicode alternatives, unified eligibility and stable compact IDs.
use astroterm::catalog::{load_athyg_catalog, StarId};
use astroterm::points::star_label;
use astroterm::progress::NoProgress;
use astroterm::sky::{build_sky_tables, prepare_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint};
use astroterm::timing::StepTimes;

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
    load_athyg_catalog(&path, &mut astroterm::progress::Report::quiet()).unwrap()
}

#[test]
fn labels_are_shared_and_cached_without_changing_source_ids() {
    let source = source();
    assert_eq!(source.stars.last().unwrap().id, StarId(7)); // skipped source row still occupies an ID
    let prepared = prepare_catalog(&source, &mut astroterm::progress::Report::quiet()).unwrap();
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
    write_cached_catalog(&path, &prepared, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    let restored = load_cached_catalog(&path, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(restored, prepared);
    assert_eq!(restored.catalog.names.get_for_mode(name(1), false), Some("Alp2 Cen"));
}

#[test]
fn identifiers_and_proper_names_follow_the_same_label_rule() {
    let source = source();
    let prepared = prepare_catalog(&source, &mut astroterm::progress::Report::quiet()).unwrap();
    let tables = build_sky_tables(&prepared, &mut astroterm::progress::Report::live(&mut NoProgress(std::io::sink()), &mut StepTimes::default())).unwrap();
    for (id, unicode, expected) in [(0, false, "Named 星"), (1, false, "Alp2 Cen"), (1, true, "α² Cen"), (2, false, "HIP 42")] {
        let (table, row) = tables.stars.iter()
            .find_map(|(_, _, table)| (0..table.len()).find(|&row| table.id(row) == StarId(id)).map(|row| (table, row)))
            .unwrap_or_else(|| panic!("star {id} is in a group table"));
        assert_eq!(star_label(table.labels(), row, &tables.names, unicode), expected);            // the text step 6 draws
    }
}

#[test]
fn ids_reject_overflow_and_preserve_the_maximum_without_renumbering() {
    assert_eq!(StarId::try_from_index(u64::from(u32::MAX)).unwrap(), StarId(u32::MAX));
    assert!(StarId::try_from_index(u64::from(u32::MAX) + 1).is_err());
    let mut source = source();
    source.stars.truncate(1);
    source.stars[0].id = StarId(u32::MAX);
    let prepared = prepare_catalog(&source, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(prepared.catalog.stars.id(0), StarId(u32::MAX));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prepared");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(load_cached_catalog(&path, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap().catalog.stars.id(0), StarId(u32::MAX));
}
