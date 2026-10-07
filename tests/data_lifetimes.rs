//! Preparation can be discarded without changing shared inputs or subsequent observations.
use astroterm::{astro::{J2000, Observer}, cli::{Arguments, build_config}, model::{SkyRegion, FrameTime},
    sky::{prepare_owned_catalog, write_cached_catalog, load_cached_catalog, catalog_fingerprint},
    state::ApplicationState, timing::StepTimes};
use clap::Parser;
use std::sync::Arc;

fn state() -> ApplicationState {
    let config = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
    ApplicationState::new(config, StepTimes::default())
}

fn observe(state: &mut ApplicationState, utc: f64) {
    let time = FrameTime::from_utc(utc);
    let cache = &mut state.cache;
    astroterm::sky::update_simulation(&mut cache.simulation, time, &[], &mut state.timings).unwrap();
    let mut observer = astroterm::sky::prepare_cached_observer(&mut cache.observation, &cache.simulation, time, Observer::default()).unwrap();
    astroterm::sky::prepare_cached_light_time(&mut cache.observation, &mut cache.simulation, &mut observer, &mut state.timings).unwrap();
    astroterm::sky::observe_cached_sky(&mut cache.observation, &cache.simulation, &observer, 5.0, false, SkyRegion::All, &mut cache.sky, &mut state.timings).unwrap();
}

#[test]
fn cold_and_warm_cleanup_preserve_rows_shared_identity_and_frame_results() {
    let source = prepare_owned_catalog(astroterm::catalog::load_embedded_catalog().unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &source, &fingerprint).unwrap();
    let loaded = load_cached_catalog(&path, &fingerprint).unwrap();
    assert_eq!(source, loaded);
    for prepared in [source, loaded] {
        let mut app = state();
        app.replace_catalog(prepared);
        let catalog = app.persistent.catalog.clone();
        let columns = catalog.stars.columns();
        let stars_pointer = columns.u0.as_ptr();
        let rows: Vec<_> = catalog.stars.iter().collect();
        assert_eq!(app.preparation().unwrap().motion_bounds().len(), rows.len());
        astroterm::sky::prepare_observation_catalog(&mut app.cache.observation, catalog.clone(), &mut app.timings);
        observe(&mut app, J2000);
        let expected = app.cache.sky.stars.clone();
        let stats = app.cache.observation.stats();
        app.free_preparation_only_data();
        app.free_preparation_only_data();
        assert!(app.preparation().is_none()); // dropping the sole preparation owner drops its Vec allocation
        assert!(Arc::ptr_eq(&catalog, &app.persistent.catalog));
        assert!(Arc::ptr_eq(&catalog, &app.cache.sky.catalog));
        assert_eq!(app.persistent.catalog.stars.columns().u0.as_ptr(), stars_pointer);
        assert_eq!(app.persistent.catalog.stars.iter().collect::<Vec<_>>(), rows);
        assert_eq!(app.cache.observation.stats(), stats);
        observe(&mut app, J2000);
        assert_eq!(app.cache.sky.stars, expected);
        assert!(app.cache.observation.stats().hits > stats.hits);
        observe(&mut app, J2000 + 1.0);
        observe(&mut app, astroterm::astro::COMPUTATIONAL_INTERVAL.end_tt + 365.25);
        assert!(app.cache.sky.selection.brute_force);
        let mut report = Vec::new();
        app.write_tables(&mut report, Some("after cleanup")).unwrap();
        let report = String::from_utf8(report).unwrap();
        assert!(!report.contains("motion\\_bound: f32"));
        assert!(report.contains("### preparation\n\n**Shape:** `[0]`"));
    }
}

#[test]
fn replacement_installs_fresh_preparation_and_clears_catalog_dependents() {
    let mut app = state();
    app.free_preparation_only_data();
    let prepare = || prepare_owned_catalog(astroterm::catalog::load_embedded_catalog().unwrap()).unwrap();
    app.replace_catalog(prepare());
    observe(&mut app, J2000);
    app.cache.sky.set_figure_override(Some(astroterm::sky::prepare_constellation_set(Vec::new(), 0).unwrap()));
    app.free_preparation_only_data();
    app.replace_catalog(prepare());
    assert!(app.preparation().is_some());
    assert!(app.cache.sky.figure_override().is_none());
    assert_eq!(app.cache.observation.stats().refreshes, 0);
    assert!(Arc::ptr_eq(app.cache.sky.figures(), &app.persistent.catalog.figures));
    assert_eq!(app.cache.sky.constellations().as_ptr(), app.persistent.catalog.constellations().as_ptr());
    assert!(astroterm::sky::prepare_constellation_set(vec![astroterm::model::Constellation {
        abbreviation: "Bad", segments: vec![[0, usize::MAX]],
    }], app.persistent.catalog.stars.len()).is_err());
}
