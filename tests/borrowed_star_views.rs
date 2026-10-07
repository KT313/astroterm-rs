use astroterm::astro::Vector3;
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{ObservedStar, ObservedStarView};
use std::{mem::size_of, sync::Arc};

#[test]
fn filtered_and_reordered_states_borrow_their_own_catalog_metadata() {
    let mut sky = astroterm::sky::create_sky_from_catalog(&load_embedded_catalog().unwrap()).unwrap();
    sky.stars = [100, 9, 1700, 1].map(|i| sky.stars[i].clone()).into();
    for (i, state) in sky.stars.iter_mut().enumerate() {
        state.magnitude = -0.0 + i as f64;
        state.position = Vector3 {
            x: i as f64,
            y: 2.0,
            z: 3.0,
        };
    }
    let mut other = sky.clone();
    other.stars[0].magnitude = 99.0;
    assert!(Arc::ptr_eq(&sky.catalog, &other.catalog));
    for (i, view) in sky.star_views().enumerate() {
        let full = sky.catalog.stars.get(view.source_index);
        assert!(std::ptr::eq(view.state, &sky.stars[i]));
        assert!(std::ptr::eq(view.catalog, &sky.catalog.stars));
        assert_eq!(view.id(), full.id);
        assert_eq!(view.name(), full.name);
        assert_eq!(view.display_color(), full.display_color);
        assert_eq!(sky.star_name(&view), sky.catalog.names.get(full.name));
        assert_eq!(view.magnitude, i as f64);
    }
    assert_eq!(sky.star_view(0).magnitude, 0.0);
    assert_eq!(other.star_view(0).magnitude, 99.0);
    assert!(size_of::<ObservedStar>() <= size_of::<Vector3>() + size_of::<f64>() + 2 * size_of::<usize>());
    assert_eq!(size_of::<ObservedStarView<'_>>(), 2 * size_of::<usize>());
}

#[test]
fn borrowed_star_debug_does_not_expand_the_whole_catalog() {
    let sky = astroterm::sky::create_sky_from_catalog(&load_embedded_catalog().unwrap()).unwrap();
    let debug = format!("{:?}", sky.star_view(0));
    assert!(debug.contains("ObservedStarView"));
    assert!(!debug.contains("StarStorage"));
    assert!(debug.len() < 1024);
}
