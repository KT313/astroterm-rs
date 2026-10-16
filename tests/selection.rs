//! Step 3 of the rebuilt frame (`select_regions`) against an independent full scan of every star: the selected
//! regions' brightness prefixes, with the constellation table, must contain every star the full scan places inside
//! the view. Ported from `spatial_selection.rs`; region coverage is qualified only for motion within the fixed
//! drift margin, exactly as before.
use astroterm::formulas::{COMPUTATIONAL_INTERVAL, J2000, JULIAN_YEAR_DAYS, Matrix3, Observer, Vector3, refract_direction};
use astroterm::catalog::{CatalogStar, SpaceMotion, StarId, decode_magnitude, load_embedded_catalog};
use astroterm::constants::{GRID_DEPTH, STELLAR_DRIFT_MARGIN};
use astroterm::model::{ProjectionKind, Data, View, ViewCenter, hash_direction};
use astroterm::observation::{Observation, observe};
use astroterm::projection::{prepare_camera, project_camera};
use astroterm::selection::select_regions;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

fn build(catalog: astroterm::catalog::Catalog) -> Data {
    let prepared = astroterm::sky::prepare_owned_catalog(catalog, &mut astroterm::progress::Report::quiet()).unwrap();
    astroterm::sky::build_sky_tables(&prepared, &mut astroterm::progress::Report::quiet()).unwrap()
}

/// BSC5 plus 4000 random stars, a tenth of them fast movers, as in the old fixture.
fn tables() -> Arc<Data> {
    static TABLES: OnceLock<Arc<Data>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut catalog = load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap();
        let mut seed = 17_u64;
        let mut uniform = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 11) as f64 / (1_u64 << 53) as f64
        };
        for i in 0..4000 {
            let ra = uniform() * std::f64::consts::TAU;
            let dec = (2.0 * uniform() - 1.0).asin();
            let position = astroterm::formulas::Equatorial { right_ascension: ra, declination: dec }.to_unit_vector();
            let speed = if i % 10 == 0 { 0.001 } else { 1e-8 };
            let velocity = Vector3 { x: (uniform() - 0.5) * speed, y: (uniform() - 0.5) * speed, z: (uniform() - 0.5) * speed };
            catalog.stars.push(CatalogStar {
                id: StarId(10000 + i), space_motion: Some(SpaceMotion { distance_pc: 1.0, position, velocity }), hr: None, name: None, designation: None,
                right_ascension: ra, declination: dec, ra_motion: 0.0, ra_motion_cos_dec: 0.0, dec_motion: 0.0,
                magnitude: uniform() * 12.0, spectral_type: *b"G2", color_index: None, has_data: true,
            });
        }
        Arc::new(build(catalog))
    }).clone()
}

/// Every star of every table: id, motion, catalog magnitude.
fn all_stars(tables: &Data) -> Vec<(u32, astroterm::formulas::StellarMotion, f64)> {
    let mut stars = Vec::new();
    for (_, _, table) in tables.star_tables() {
        for row in 0..table.len() { stars.push((table.id(row).0, table.motion(row), decode_magnitude(table.columns().magnitude[row]))); }
    }
    let constellations = &tables.constellation_stars;
    for row in 0..constellations.len() { stars.push((constellations.id(row).0, constellations.motion(row), decode_magnitude(constellations.columns().magnitude[row]))); }
    stars
}

/// The stars the selection offers: the brightness prefixes of the selected regions plus the constellation table.
fn candidates(tables: &Data, observation: &Observation, threshold: f64) -> (BTreeSet<u32>, bool) {
    let selected = select_regions(&tables.star_regions, &observation.patch, observation.time_ms, &mut astroterm::time::StepTimes::default());
    let mut ids = BTreeSet::new();
    for &region in &selected.regions {
        for table in tables.star_regions[region].speed_groups.iter().map(|group| &group.stars) {
            for row in 0..table.bright_count(threshold) { ids.insert(table.id(row).0); }
        }
    }
    for row in 0..tables.constellation_stars.len() { ids.insert(tables.constellation_stars.id(row).0); }
    (ids, selected.whole_sky)
}

/// The stars a full scan places inside the view at or above the threshold; `drift_limited` drops stars that moved
/// more than the drift margin, the documented limit of region selection in views under 300°.
fn visible_by_full_scan(tables: &Data, observation: &Observation, view: &View, threshold: f64, drift_limited: bool) -> BTreeSet<u32> {
    let years = (astroterm::formulas::AstronomicalTime::from_time_ms(observation.time_ms).tt_jd - J2000) / JULIAN_YEAR_DAYS;
    let camera = prepare_camera(view);
    all_stars(tables).into_iter().filter_map(|(id, motion, magnitude)| {
        let sample = motion.evaluate(years, magnitude);
        if astroterm::catalog::decode_magnitude(astroterm::catalog::magnitude_code(sample.magnitude)) > threshold { return None; }
        let drift = motion.u0.cross(sample.direction).length().atan2(motion.u0.dot(sample.direction));
        if drift_limited && drift > STELLAR_DRIFT_MARGIN { return None; }
        let apparent = observation.apparent_star_direction(sample.direction);
        project_camera(camera, apparent).is_some_and(|point| point.is_visible()).then_some(id)
    }).collect()
}

fn compare(date: f64, view: View, threshold: f64, refraction: bool, latitude: f64, longitude: f64) {
    let tables = tables();
    let observation = observe(&Observer { latitude, longitude }, refraction, &view, astroterm::formulas::utc_jd_to_time_ms(date).unwrap(), astroterm::formulas::evaluate_earth_state(astroterm::formulas::utc_jd_to_time_ms(date).unwrap()), &mut astroterm::time::StepTimes::default());
    let (offered, whole_sky) = candidates(&tables, &observation, threshold);
    let visible = visible_by_full_scan(&tables, &observation, &view, threshold, !whole_sky && view.fov_degrees < 300.0);
    let missing: Vec<_> = visible.difference(&offered).collect();
    assert!(missing.is_empty(), "{date} fov {} threshold {threshold} refraction {refraction}: not offered {missing:?}", view.fov_degrees);
    if !COMPUTATIONAL_INTERVAL.contains(astroterm::formulas::AstronomicalTime::from_time_ms(observation.time_ms).tt_jd) { assert!(whole_sky); }
}

fn check_boundary_date(date: f64) {
    for fov in [1.0, 180.0, 300.0, 359.0, 360.0] {
        for refraction in [false, true] {
            let view = View { fov_degrees: fov, projection: ProjectionKind::Equidistant, center: ViewCenter::Facing { azimuth: 0.0, tilt: 0.0 } };
            compare(date, view, 5.0, refraction, 0.0, 0.0);
        }
    }
}

#[test]
fn views_before_interval_match_full_scan_within_drift_margin() { check_boundary_date(COMPUTATIONAL_INTERVAL.start_tt - 1.0); }
#[test]
fn views_at_interval_start_match_full_scan_within_drift_margin() { check_boundary_date(COMPUTATIONAL_INTERVAL.start_tt); }
#[test]
fn views_at_j2000_match_full_scan_within_drift_margin() { check_boundary_date(J2000); }
#[test]
fn views_just_before_interval_end_match_full_scan_within_drift_margin() { check_boundary_date(COMPUTATIONAL_INTERVAL.end_tt.next_down()); }
#[test]
fn views_at_interval_end_match_full_scan_within_drift_margin() { check_boundary_date(COMPUTATIONAL_INTERVAL.end_tt); }

// Four independent property tests, 128 cases in total, covering both projections with and without refraction.
macro_rules! test_random_regions {
    ($name:ident, $projection:expr, $refraction:expr) => {
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(32))]
            #[test]
            fn $name(
                date in (COMPUTATIONAL_INTERVAL.start_tt - 365250.0)..(COMPUTATIONAL_INTERVAL.end_tt + 365250.0),
                azimuth in 0.0_f64..std::f64::consts::TAU,
                tilt in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2,
                fov in 1.0_f64..359.0, threshold in -2.0_f64..14.0,
                latitude in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2,
                longitude in -std::f64::consts::PI..std::f64::consts::PI,
            ) {
                compare(date, View { center: ViewCenter::Facing { azimuth, tilt }, fov_degrees: fov, projection: $projection }, threshold, $refraction, latitude, longitude);
            }
        }
    };
}
test_random_regions!(random_stereographic_airless_matches_full_scan_within_drift_margin, ProjectionKind::Stereographic, false);
test_random_regions!(random_stereographic_refracted_matches_full_scan_within_drift_margin, ProjectionKind::Stereographic, true);
test_random_regions!(random_equidistant_airless_matches_full_scan_within_drift_margin, ProjectionKind::Equidistant, false);
test_random_regions!(random_equidistant_refracted_matches_full_scan_within_drift_margin, ProjectionKind::Equidistant, true);

/// The concrete failure recorded in `spatial_selection.proptest-regressions`.
#[test]
fn saved_stereographic_selection_regression_matches_full_scan_within_drift_margin() {
    let view = View { center: ViewCenter::Facing { azimuth: 0.0, tilt: 0.0 }, fov_degrees: 170.70431445631192, projection: ProjectionKind::Stereographic };
    compare(0.0, view, 5.381495729624377, false, 0.0, 0.0);
}

/// Three stars of magnitude 4.999, 5.0 and 5.001 at a cube seam, at the pole, 1° below the horizon under refraction
/// and 0.001° above it: the selection offers them and the threshold keeps exactly ids 0 and 1.
#[test]
fn seam_threshold_horizon_and_view_edge_cases_are_not_culled() {
    use astroterm::formulas::Horizontal;
    for (direction, view, refraction) in [
        (Vector3 { x: 1.0, y: 1.0, z: 1.0 }.normalized(), View::default(), false),
        (Vector3 { x: 0.0, y: 0.0, z: 1.0 }, View::default(), false),
        (Horizontal { azimuth: 0.0, altitude: -1_f64.to_radians() }.to_unit_vector(), View { center: ViewCenter::Facing { azimuth: 0.0, tilt: 89.5_f64.to_radians() }, ..View::default() }, true),
        (Horizontal { azimuth: 0.0, altitude: 0.001_f64.to_radians() }.to_unit_vector(), View::default(), false),
    ] {
        let mut parsed = load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap();
        let template = parsed.stars[0].clone();
        parsed.stars.clear();
        parsed.constellations.clear();
        parsed.hr_representatives.clear();
        for (i, magnitude) in [4.999, 5.0, 5.001].into_iter().enumerate() {
            let mut star = template.clone();
            star.id = StarId(i as u32);
            star.has_data = true;
            star.magnitude = magnitude;
            star.right_ascension = direction.y.atan2(direction.x);
            star.declination = direction.z.asin();
            star.space_motion = Some(SpaceMotion { distance_pc: 1.0, position: direction, velocity: Vector3::default() });
            parsed.stars.push(star);
        }
        let tables = build(parsed);
        let mut observation = observe(&Observer::default(), refraction, &view, astroterm::formulas::utc_jd_to_time_ms(J2000).unwrap(), astroterm::formulas::evaluate_earth_state(astroterm::formulas::utc_jd_to_time_ms(J2000).unwrap()), &mut astroterm::time::StepTimes::default());
        observation.to_horizon = Matrix3::IDENTITY;                                        // the catalog axes are the horizon axes in this fixture
        observation.to_catalog = Matrix3::IDENTITY;
        observation.observer.velocity = Vector3::default();
        observation.patch.center = view.center_direction().to_unit_vector();
        let (offered, _) = candidates(&tables, &observation, 5.0);
        assert_eq!(offered, BTreeSet::from([0, 1]));
        let visible = visible_by_full_scan(&tables, &observation, &view, 5.0, false);
        assert_eq!(visible, BTreeSet::from([0, 1]));
        let position = if refraction { refract_direction(direction) } else { direction };
        assert!(project_camera(prepare_camera(&view), position).is_some_and(|point| point.is_visible()));
    }
}

/// A fast star stays in the region of its catalog direction; after 1000 years a cone around its moved direction no
/// longer finds it (the documented limitation), while full-sky selection still offers both stars.
#[test]
fn fast_stars_use_ordinary_regions_and_large_drift_has_no_special_selection() {
    let mut source = load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap();
    let mut fast = source.stars[0].clone();
    fast.has_data = true;
    fast.id = StarId(1);
    fast.magnitude = 4.0;
    fast.space_motion = Some(SpaceMotion { distance_pc: 1.0, position: Vector3 { x: 1.0, y: 0.0, z: 0.0 }, velocity: Vector3 { x: 0.0, y: 0.001, z: 0.0 } });
    let mut stationary = fast.clone();
    stationary.id = StarId(2);
    stationary.space_motion.as_mut().unwrap().velocity = Vector3::default();
    source.stars = vec![fast, stationary];
    source.constellations.clear();
    source.hr_representatives.clear();
    let tables = build(source);
    let place = |id: u32| tables.star_tables().find_map(|(region, group, table)| (0..table.len()).find(|&row| table.id(row).0 == id).map(|row| (region, group, row))).unwrap();
    let ((region, group, row), (slow_region, slow_group, _)) = (place(1), place(2));
    assert_eq!((region, group, slow_group), (slow_region, 2, 0));                               // same region, one fast and one slow table
    let motion = tables.star_table(region, group).motion(row);
    assert_eq!(hash_direction(GRID_DEPTH, motion.u0), region);                                   // speed does not change the region
    assert!(motion.motion_bound() > STELLAR_DRIFT_MARGIN);

    let time = astroterm::formulas::utc_jd_to_time_ms(J2000 + 1000.0 * JULIAN_YEAR_DAYS).unwrap();
    let cone = |center: Vector3, threshold: f64| {
        let mut observation = observe(&Observer::default(), false, &View { fov_degrees: 2.0, ..View::default() }, time, astroterm::formulas::evaluate_earth_state(time), &mut astroterm::time::StepTimes::default());
        observation.patch.center = center;
        candidates(&tables, &observation, threshold).0
    };
    assert_eq!(cone(motion.u0, 5.0), BTreeSet::from([1, 2]));
    assert!(cone(motion.u0, 3.0).is_empty());                                                    // the brightness prefix still applies
    let moved = motion.evaluate(1000.0, 4.0).direction;
    assert!(cone(moved, 5.0).is_empty());                                                        // documented limitation
    let wide = observe(&Observer::default(), false, &View { fov_degrees: 360.0, ..View::default() }, time, astroterm::formulas::evaluate_earth_state(time), &mut astroterm::time::StepTimes::default());
    assert_eq!(candidates(&tables, &wide, 5.0).0, BTreeSet::from([1, 2]));
}
