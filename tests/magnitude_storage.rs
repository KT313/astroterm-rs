//! Quantized catalog magnitudes, conservative pruning and persisted clipping diagnostics.
use astroterm::{
    catalog::{Catalog, StarNames, load_athyg_catalog, MagnitudeClipping, decode_magnitude},
    catalog::datasets::{Dataset, DatasetDirectories},
    model::{ObservedSky, SkyRegion},
    sky::{prepare_owned_catalog, load_sky_catalog, load_cached_catalog, write_cached_catalog, catalog_fingerprint},
};
use std::sync::Arc;

fn load_csv(text: &str) -> Result<Catalog, astroterm::catalog::CatalogError> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stars.csv");
    std::fs::write(&path, text).unwrap();
    load_athyg_catalog(&path)
}

#[test]
fn source_endpoints_remain_f64_until_checked_and_sun_stays_excluded() {
    let parsed = load_csv("ra,dec,mag\n0,0,-26\n0,0,-10\n0,0,55.535\n").unwrap();
    assert_eq!(parsed.stars.len(), 2);
    assert_eq!(parsed.stars[1].magnitude, 55.535);
    let prepared = prepare_owned_catalog(parsed).unwrap();
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
    let mut codes = prepared.catalog.stars.columns().magnitude.to_vec();
    codes.sort();
    assert_eq!(codes, [0, 65535]);
    for magnitude in ["-10.00000001", "55.53500001", "-20", "nan", "inf"] {
        let error = load_csv(&format!("ra,dec,mag\n0,0,{magnitude}\n")).unwrap_err();
        assert!(error.to_string().contains("line 2"), "{error}");
    }
    let mut source = astroterm::catalog::load_embedded_catalog().unwrap();
    source.stars[0].magnitude = 55.535_f64.next_up();
    assert!(prepare_owned_catalog(source).unwrap_err().to_string().contains("outside the allowed range"));
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
    let prepared = prepare_owned_catalog(parsed).unwrap();
    let stars = &prepared.catalog.stars;
    assert_eq!(stars.columns().id, [2, 1, 0]); // all three now tie, stored brightest-first with descending ID
    assert!(stars.columns().magnitude.iter().all(|&code| decode_magnitude(code) == 5.0));
    assert_eq!(prepared.catalog.count_bright_stars(4.9999), 0);
    assert_eq!(prepared.catalog.count_bright_stars(5.0), 3);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache");
    let fingerprint = catalog_fingerprint();
    write_cached_catalog(&path, &prepared, &fingerprint).unwrap();
    assert_eq!(load_cached_catalog(&path, &fingerprint).unwrap(), prepared);
}

#[test]
fn clipped_minimum_passes_grid_and_candidate_checks_before_unclipped_current_filter() {
    use astroterm::{astro::{J2000, JULIAN_YEAR_DAYS, Matrix3, Observer, Vector3}, model::FrameTime,
        sky::{observe_sky, prepare_observation, update_solar_system}, state::SimulationState, timing::StepTimes};
    let parsed = load_csv("ra,dec,mag,dist,rv\n0,0,-9.9,10,-100\n0,0,-10,,\n").unwrap();
    let catalog = Arc::new(prepare_owned_catalog(parsed).unwrap().catalog);
    // Both encoded-zero rows pass an arbitrarily bright early threshold; final brightness still decides.
    assert_eq!(catalog.count_bright_stars(-50.0), 2);
    let mut sky = ObservedSky::new(catalog);
    for (years, drawn) in [(0.0, 0), (9000.0, 1)] {
        let tt = J2000 + years * JULIAN_YEAR_DAYS;
        let time = FrameTime { utc: tt, ut1: tt, tt };
        let mut simulation = SimulationState::exact();
        update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
        let mut observer = prepare_observation(&mut simulation, time, Observer::default()).unwrap();
        observer.inertial_to_horizon = Matrix3::IDENTITY;
        observer.state.velocity = Vector3::default();
        observe_sky(&simulation, &observer, -10.05, false, SkyRegion::All, &mut sky, &mut StepTimes::default()).unwrap();
        assert_eq!(sky.stars.iter().filter(|s| s.drawable).count(), drawn);
        assert_eq!(sky.selection.candidates, 2);
        if drawn == 1 { assert!(sky.stars[0].magnitude < -10.05); }
    }
}

#[test]
fn clipping_notice_coexists_with_date_notice_and_cache_tracks_it() {
    use astroterm::{canvas::Canvas, model::{View, ProjectionViewport, RenderOptions}, projection::project_sky,
        scene::draw_characters, state::SceneCache};
    let parsed = load_csv("ra,dec,mag,dist,rv\n0,0,-9.9,10,-100\n").unwrap();
    let mut clipped = astroterm::sky::create_sky_from_catalog(&parsed).unwrap();
    clipped.outside_accuracy_range = true;
    clipped.stars.clear(); // identical raster inputs apart from the new notice
    let mut plain = astroterm::sky::create_sky_from_catalog(&Catalog::new(Vec::new(), StarNames::default(), Vec::new())).unwrap();
    plain.outside_accuracy_range = true;
    let view = View::default();
    let viewport = ProjectionViewport { width: 140, height: 8 };
    let options = RenderOptions { unicode: true, braille: false, color: false, constellations: false, grid: false,
        magnitude_threshold: -50.0, dynamic_names: false };
    let mut cache = SceneCache::default();
    let mut canvas = Canvas::new(8, 140);
    for sky in [&plain, &clipped, &clipped, &plain] {
        let data = project_sky(sky, &view, viewport);
        let projected = data.view(sky);
        draw_characters(&mut cache, &mut canvas, &projected, &options, 0.0);
        let text = canvas.to_lines().join("\n");
        assert_eq!(text.contains("Stored brightness bounds clipped"), projected.magnitude_clipping().any());
        if projected.magnitude_clipping().any() {
            let lines = canvas.to_lines();
            assert!(lines[6].starts_with("Stored brightness bounds clipped"));
            assert!(lines[7].starts_with(astroterm::astro::accuracy::ACCURACY_WARNING));
        }
    }
}

#[test]
fn runtime_magnitude_can_grow_beyond_the_storage_maximum() {
    let parsed = load_csv("ra,dec,mag,dist,rv\n0,0,55.535,10,100\n").unwrap();
    let prepared = prepare_owned_catalog(parsed).unwrap();
    let star = prepared.catalog.stars.get(0);
    assert_eq!(star.magnitude, 55.535);
    assert!(star.motion.evaluate(9000.0, star.magnitude).magnitude > 55.535);
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
}

#[test]
fn stationary_minimum_magnitude_does_not_warn_for_direction_quantization() {
    let parsed = load_csv("ra,dec,mag,dist,rv\n3,35.26438968,-10,10,0\n").unwrap();
    let prepared = prepare_owned_catalog(parsed).unwrap();
    let star = prepared.catalog.stars.get(0);
    assert!(!prepared.catalog.stars.magnitude_clipping().any());
    assert_eq!(star.brightness_key, -10.0);
    for years in [-9900.0, 0.0, 10000.0] { assert_eq!(star.motion.evaluate(years, star.magnitude).magnitude, -10.0); }
}
