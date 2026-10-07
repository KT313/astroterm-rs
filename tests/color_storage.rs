//! Color indices survive catalog preparation, compact storage and disk-cache loading without source metadata.
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::StarColor;
use astroterm::sky::{prepare_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint};

#[test]
fn every_palette_entry_survives_preparation_and_cache_loading() {
    let mut source = load_embedded_catalog().unwrap();
    source.stars.truncate(8);
    let cases = [(*b"  ", None, StarColor::Default), (*b"W9", Some(2.0), StarColor::HotBlue),
        (*b"B0", None, StarColor::BlueWhite), (*b"A0", None, StarColor::White),
        (*b"F0", None, StarColor::YellowWhite), (*b"G0", None, StarColor::Yellow),
        (*b"??", Some(0.8), StarColor::Orange), (*b"  ", Some(1.4), StarColor::RedOrange)];
    for (star, &(spectral, bv, _)) in source.stars.iter_mut().zip(&cases) {
        star.spectral_type = spectral;
        star.color_index = bv;
    }
    let prepared = prepare_catalog(&source).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("colors.catalog");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    let cached = load_cached_catalog(&path, &fingerprint).unwrap();
    assert_eq!(cached, prepared);
    for data in [&prepared, &cached] {
        let columns = data.catalog.stars.columns();
        for i in 0..data.catalog.stars.len() {
            let source_index = source.stars.iter().position(|s| s.id == data.catalog.stars.id(i)).unwrap();
            let color = cases[source_index].2;
            assert_eq!(columns.display_color[i], color.index());
            assert_eq!(data.catalog.stars.display_color(i), color);
            assert_eq!(data.catalog.stars.get(i).display_color, color);
            assert!(data.catalog.star_exceptions.is_empty()); // known/missing B-V requires no runtime flags
        }
    }
}
