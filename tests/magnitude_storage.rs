//! Quantized catalog magnitudes, conservative pruning and persisted clipping diagnostics.
use astroterm::{
    catalog::{Catalog, StarNames, load_athyg_catalog, MagnitudeClipping, decode_magnitude, magnitude_code},
    catalog::datasets::{Dataset, DatasetDirectories},
    constants::MAST_PARALLEL_BLOCKS,
    model::{PreparedCatalog, Data},
    progress::NoProgress,
    sky::{build_sky_tables, prepare_owned_catalog, load_cached_catalog, write_cached_catalog, catalog_fingerprint},
    time::StepTimes,
};

/// The loader without progress display or timings, with its notices collected.
fn load_sky_catalog(dataset: Option<&Dataset>, dirs: &DatasetDirectories, notices: &mut Vec<u8>) -> std::io::Result<PreparedCatalog> {
    astroterm::sky::load_sky_catalog(dataset, MAST_PARALLEL_BLOCKS, dirs, &mut astroterm::progress::Report::live(&mut NoProgress(notices), &mut StepTimes::default()))
}

fn tables(prepared: &PreparedCatalog) -> Data {
    build_sky_tables(prepared, &mut astroterm::progress::Report::live(&mut NoProgress(std::io::sink()), &mut StepTimes::default())).unwrap()
}

fn load_csv(text: &str) -> Result<Catalog, astroterm::catalog::CatalogError> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stars.csv");
    std::fs::write(&path, text).unwrap();
    load_athyg_catalog(&path, &mut astroterm::progress::Report::quiet())
}

#[test]
fn source_endpoints_remain_f64_until_checked_and_sun_stays_excluded() {
    let parsed = load_csv("ra,dec,mag\n0,0,-26\n0,0,-10\n0,0,55.535\n").unwrap();
    assert_eq!(parsed.stars.len(), 2);
    assert_eq!(parsed.stars[1].magnitude, 55.535);
    let prepared = prepare_owned_catalog(parsed, &mut astroterm::progress::Report::quiet()).unwrap();
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
    let mut codes = prepared.catalog.stars.columns().magnitude.to_vec();
    codes.sort();
    assert_eq!(codes, [0, 65535]);
    for magnitude in ["-10.00000001", "55.53500001", "-20", "nan", "inf"] {
        let error = load_csv(&format!("ra,dec,mag\n0,0,{magnitude}\n")).unwrap_err();
        assert!(error.to_string().contains("line 2"), "{error}");
    }
    let mut source = astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap();
    source.stars[0].magnitude = 55.535_f64.next_up();
    assert!(prepare_owned_catalog(source, &mut astroterm::progress::Report::quiet()).unwrap_err().to_string().contains("outside the allowed range"));
}

#[test]
fn clipped_bounds_survive_cache_and_emit_one_console_warning_per_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stars.csv");
    std::fs::write(&path, "ra,dec,mag,dist,rv\n0,0,-9.9,10,-100\n1,1,-9.9,10,-100\n2,2,5,10,0\n").unwrap();
    let dirs = DatasetDirectories { data: None, cache: Some(directory.path().join("cache")) };
    let mut previous = None;
    for _ in 0..2 {
        let mut notices = Vec::new();
        let prepared = load_sky_catalog(Some(&Dataset::Path(path.clone())), &dirs, &mut notices).unwrap();
        assert_eq!(prepared.catalog.stars.magnitude_clipping(), MagnitudeClipping { lower: 2, upper: 0 });
        let text = String::from_utf8(notices).unwrap();
        assert_eq!(text.matches("Stored brightness bounds clipped").count(), 1);
        assert!(text.contains("Lower clips: 2; upper clips: 0"));
        assert_eq!(prepared.catalog.count_bright_stars(-10.05), 2);
        if let Some(previous) = &previous { assert_eq!(&prepared, previous); }
        previous = Some(prepared);
    }
    let mut notices = Vec::new();
    load_sky_catalog(None, &dirs, &mut notices).unwrap();
    assert!(!String::from_utf8(notices).unwrap().contains("clipped"));
}

#[test]
fn quantization_changes_ties_but_keeps_id_order_and_decoded_runtime_values() {
    let mut parsed = load_csv("ra,dec,mag\n0,0,5.0001\n0,0,5.0002\n0,0,4.9998\n").unwrap();
    parsed.names = StarNames::default();
    let prepared = prepare_owned_catalog(parsed, &mut astroterm::progress::Report::quiet()).unwrap();
    let stars = &prepared.catalog.stars;
    assert_eq!(stars.columns().id, [2, 1, 0]); // all three now tie, stored brightest-first with descending ID
    assert!(stars.columns().magnitude.iter().all(|&code| decode_magnitude(code) == 5.0));
    assert_eq!(prepared.catalog.count_bright_stars(4.9999), 0);
    assert_eq!(prepared.catalog.count_bright_stars(5.0), 3);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap();
    assert_eq!(load_cached_catalog(&path, &fingerprint, &mut astroterm::progress::Report::quiet()).unwrap(), prepared);
}

#[test]
fn clipped_minimum_passes_the_brightness_prefix_before_the_saturating_current_magnitude() {
    use astroterm::astro::{J2000, JULIAN_YEAR_DAYS};
    use astroterm::regions::{UNPLACED, rows_wanted};
    let parsed = load_csv("ra,dec,mag,dist,rv\n0,0,-9.9,10,-100\n0,0,-10,,\n").unwrap();
    let prepared = prepare_owned_catalog(parsed, &mut astroterm::progress::Report::quiet()).unwrap();
    // Both encoded-zero rows pass an arbitrarily bright early threshold; the current magnitude still decides.
    assert_eq!(prepared.catalog.count_bright_stars(-50.0), 2);
    let tables = tables(&prepared);
    for (years, drawn) in [(0.0, 1), (9000.0, 2)] {                                        // the current magnitude saturates at -10.000, so the threshold -10 draws it
        let tt = J2000 + years * JULIAN_YEAR_DAYS;
        let mut simulations = tables.clone();
        let mut codes = Vec::new();
        for (region, group, table) in tables.star_tables() {
            let rows = rows_wanted(table, tt, -10.0);
            assert_eq!(rows, table.len());                                                   // the stored bounds admit both rows at every date
            (simulations.star_regions[region].speed_groups[group].update_simulation(astroterm::astro::utc_jd_to_time_ms(tt).unwrap(), tt, rows_wanted(table, tt, -10.0)));
            codes.extend(simulations.star_regions[region].speed_groups[group].simulation.magnitudes().iter().take(rows).copied().filter(|&code| code != UNPLACED));
        }
        assert_eq!(codes.iter().filter(|&&code| code <= magnitude_code(-10.0)).count(), drawn);
        if drawn == 2 { assert!(codes.iter().all(|&code| code == 0)); }                      // the approaching star is calculated brighter and saturates at -10.000
    }
}

#[test]
fn clipping_notice_coexists_with_date_notice_and_the_tables_carry_it() {
    use astroterm::astro::{J2000, Observer, accuracy::ACCURACY_WARNING};
    use astroterm::catalog::MAGNITUDE_CLIPPING_WARNING;
    use astroterm::model::{RenderOptions, View};
    use astroterm::observation::observe;
    use astroterm::points::{PointInputs, draw_points};
    use astroterm::projection::{Frame, SkyArea};
    use astroterm::regions::refresh_regions;
    use astroterm::selection::select_regions;
    use astroterm::solar_system::simulate_solar_system;
    let notices = |tables: &Data, utc: f64| {
        let time = astroterm::astro::utc_jd_to_time_ms(utc).unwrap();
        let view = View::default();
        let settings = RenderOptions { unicode: true, braille: false, color: false, constellations: false, grid: false, magnitude_threshold: -50.0, dynamic_names: false };
        let solar_system = simulate_solar_system(tables, time, &mut astroterm::time::StepTimes::default());
        let observation = observe(&Observer::default(), false, &view, time, &mut astroterm::time::StepTimes::default());
        let selected = select_regions(&tables.star_regions, &observation, &mut astroterm::time::StepTimes::default());
        let mut caches = tables.clone();
        refresh_regions(&mut caches, &selected, time, &view, &settings, &mut astroterm::time::StepTimes::default());
        let mut frame = Frame::new(140, 60, SkyArea::centred(140, 60), (10, 20));
        let inputs = PointInputs { solar_system: &solar_system, data: &caches, selected: &selected, panel: &[], notice: None };
        draw_points(&mut frame, &inputs, &observation, &view, &settings, &mut astroterm::time::StepTimes::default()).notices
    };
    let clipped = tables(&prepare_owned_catalog(load_csv("ra,dec,mag,dist,rv\n0,0,-9.9,10,-100\n").unwrap(), &mut astroterm::progress::Report::quiet()).unwrap());
    let plain = tables(&prepare_owned_catalog(Catalog::new(Vec::new(), StarNames::default(), Vec::new()), &mut astroterm::progress::Report::quiet()).unwrap());
    assert!(clipped.clipping.any() && !plain.clipping.any());
    let far = J2000 + 20_000.0 * 365.25;                                                      // outside every validated interval
    assert_eq!(notices(&clipped, far), [ACCURACY_WARNING, MAGNITUDE_CLIPPING_WARNING]);         // the clipping notice sits above the date notice
    assert_eq!(notices(&plain, far), [ACCURACY_WARNING]);
    assert_eq!(notices(&clipped, J2000), [MAGNITUDE_CLIPPING_WARNING]);
    assert!(notices(&plain, J2000).is_empty());
}

#[test]
fn runtime_magnitude_beyond_the_storage_maximum_saturates() {
    let parsed = load_csv("ra,dec,mag,dist,rv\n0,0,55.535,10,100\n").unwrap();
    let prepared = prepare_owned_catalog(parsed, &mut astroterm::progress::Report::quiet()).unwrap();
    let star = prepared.catalog.stars.get(0);
    assert_eq!(star.magnitude, 55.535);
    assert_eq!(star.motion.evaluate(9000.0, star.magnitude).magnitude, u16::MAX); // the calculated value is fainter; the sample code clamps and is not a clipping event
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
}

#[test]
fn stationary_minimum_magnitude_does_not_warn_for_direction_quantization() {
    let parsed = load_csv("ra,dec,mag,dist,rv\n3,35.26438968,-10,10,0\n").unwrap();
    let prepared = prepare_owned_catalog(parsed, &mut astroterm::progress::Report::quiet()).unwrap();
    let star = prepared.catalog.stars.get(0);
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
    assert_eq!(star.brightness_key, -10.0);
    for years in [-9900.0, 0.0, 10000.0] { assert_eq!(star.motion.evaluate(years, star.magnitude).magnitude_value(), -10.0); }
}
