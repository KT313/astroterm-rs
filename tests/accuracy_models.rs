//! Independent phase-6 fixtures. References use DE441/ERFA components with matched TT and UT1;
//! empirical coverage is deliberately narrower than the computational/indexing interval.
use astroterm::state::{SimulationState};
use astroterm::astro::{
    self, Horizontal, Observer, Vector3,
    models::{BodyId, orientation::*},
};
use astroterm::model::{Sky, SkyRegion, FrameTime};
use astroterm::sky::{observe_sky, prepare_observation, update_simulation};
use astroterm::timing::StepTimes;
use serde_json::Value;

fn vector(v: &Value) -> Vector3 {
    Vector3 {
        x: v[0].as_f64().unwrap(),
        y: v[1].as_f64().unwrap(),
        z: v[2].as_f64().unwrap(),
    }
}
fn angle(a: Vector3, b: Vector3) -> f64 {
    a.cross(b).length().atan2(a.dot(b)).to_degrees() * 3600.0
}
fn observe(tt: f64, ut1: f64) -> Sky {
    let mut state = SimulationState::default();
    let mut times = StepTimes::default();
    // Deliberately seed the caches 59 seconds before the requested frame.
    let seed = FrameTime {
        utc: ut1 - 59.0 / 86400.0,
        ut1: ut1 - 59.0 / 86400.0,
        tt: tt - 59.0 / 86400.0,
    };
    update_simulation(&mut state, seed, &[], &mut times).unwrap();
    let time = FrameTime { utc: ut1, ut1, tt };
    update_simulation(&mut state, time, &[], &mut times).unwrap();
    let site = Observer {
        latitude: 42.3601_f64.to_radians(),
        longitude: -71.0589_f64.to_radians(),
    };
    let observer = prepare_observation(&mut state, time, site).unwrap();
    let mut sky = astroterm::sky::create_sky_from_catalog(&astroterm::catalog::load_embedded_catalog().unwrap());
    observe_sky(&state, &observer, 5.0, false, SkyRegion::All, &mut sky, &mut times).unwrap();
    sky
}
fn body(sky: &Sky, name: &str) -> Vector3 {
    if name == "Moon" {
        sky.moon.position
    } else {
        sky.planets.iter().find(|p| p.kind.name() == name).unwrap().position
    }
}

#[test]
fn precession_and_nutation_match_erfa_across_past_and_future() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reference/accuracy.json")).unwrap();
    for row in fixtures["rows"].as_array().unwrap() {
        let tt = row["tt"].as_f64().unwrap();
        let p = compute_precession_matrix(tt).matrix();
        for i in 0..3 {
            let actual = Vector3 {
                x: p.0[i][0],
                y: p.0[i][1],
                z: p.0[i][2],
            };
            assert!(angle(actual, vector(&row["precession"][i])) < astro::PRECESSION_TARGET_ARCSECONDS);
        }
        let (psi, eps) = compute_nutation(tt);
        assert!((psi - row["nutation"][0].as_f64().unwrap()).abs() < 1e-12);
        assert!((eps - row["nutation"][1].as_f64().unwrap()).abs() < 1e-12);
    }
    // Original long-term audit also includes the explicitly unsupported year 15026.
    let original: Value = serde_json::from_str(include_str!("fixtures/reference/references.json")).unwrap();
    for row in original["precession"].as_array().unwrap() {
        let tt = astro::J2000 + (row["julian_epoch_tt"].as_f64().unwrap() - 2000.0) * 365.25;
        let p = compute_precession_matrix(tt).matrix();
        for i in 0..3 {
            assert!(
                angle(
                    Vector3 {
                        x: p.0[i][0],
                        y: p.0[i][1],
                        z: p.0[i][2]
                    },
                    vector(&row["ltp_matrix"][i])
                ) < 0.01
            );
        }
    }
}

#[test]
fn horizons_body_centers_match_with_explicit_ut1_and_tt() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reference/topocentric.json")).unwrap();
    for i in 0..3 {
        let row = &fixtures["bodies"][0]["rows"][i];
        let parse = |key: &str| row[key].as_str().unwrap().parse::<f64>().unwrap();
        let tt = parse("Date_________JDTT");
        let ut1 = tt - (parse("TDB-UT") - parse("UT1-UTC")) / 86400.0; // TT≈TDB; <0.03″ rotation error near today
        let sky = observe(tt, ut1);
        for object in fixtures["bodies"].as_array().unwrap() {
            let name = object["name"].as_str().unwrap();
            let row = &object["rows"][i];
            let az = row["Azimuth_(a-app)"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                .to_radians();
            let alt = row["Elevation_(a-app)"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                .to_radians();
            let error = angle(
                body(&sky, name),
                Horizontal {
                    azimuth: az,
                    altitude: alt,
                }
                .to_unit_vector(),
            );
            eprintln!("Horizons {tt} {name}: {error:.4} arcsec");
            assert!(error < if name == "Moon" { 15.0 } else { 2.0 }, "{tt} {name}: {error}");
        }
    }
}

#[test]
fn observed_frames_match_de441_in_the_claimed_intervals_and_record_far_failures() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reference/accuracy.json")).unwrap();
    for year in [-7974.0, -2000.0, 0.0, 1900.0, 2026.0, 4026.0, 8026.0, 12026.0] {
        let epoch = astro::J2000 + (year - 2000.0) * 365.25;
        let row = fixtures["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["tt"].as_f64() == Some(epoch))
            .unwrap();
        let tt = epoch + 59.0 / 86400.0;
        let sky = observe(tt, tt);
        for object in row["bodies"].as_array().unwrap() {
            let name = object[0].as_str().unwrap();
            let error = angle(body(&sky, name), vector(&object[1]));
            let (range, class) = if name == "Moon" {
                (astro::MOON_VALIDATED_INTERVAL, astro::ObjectClass::Moon)
            } else {
                (astro::PLANET_VALIDATED_INTERVAL, astro::ObjectClass::SunAndPlanets)
            };
            if range.unwrap().contains(tt) {
                let target = astro::accuracy_target_arcseconds(class, tt).unwrap();
                assert!(error < target, "{year} {name}: {error} >= {target}");
            } else {
                assert!(error.is_finite());
            }
        }
        for (i, hr) in [7001, 5340].into_iter().enumerate() {
            let star = sky.star_views().find(|s| s.id().0 == hr).unwrap();
            assert!(angle(star.position, vector(&row["stars"][i])) < 0.1);
        }
    }
}

#[test]
fn barycentric_sun_moves_and_earth_is_not_the_earth_moon_barycenter() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reference/accuracy.json")).unwrap();
    let tt = astro::J2000 + (2026.0 - 2000.0) * 365.25;
    let row = fixtures["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["tt"].as_f64() == Some(tt))
        .unwrap();
    let states = astro::models::planets::evaluate_planets(tt);
    assert!(states[BodyId::Sun as usize].position.length() > 0.001);
    let error = ((states[BodyId::Earth as usize].position - states[BodyId::Sun as usize].position)
        - (vector(&row["states"][3]) - vector(&row["states"][0])))
    .length();
    assert!(error < 2e-6, "Heliocentric Earth error {error} AU"); // comfortably excludes a ~3e-5 AU EMB substitution
}

#[test]
fn apparent_sidereal_rotation_matches_erfa_near_today() {
    let data: Value = serde_json::from_str(include_str!("fixtures/reference/orientation.json")).unwrap();
    for row in data["rows"].as_array().unwrap() {
        let tt = row["tt"].as_f64().unwrap();
        let ut1 = row["ut1"].as_f64().unwrap();
        let eo = compute_mean_equation_of_origins(tt) - compute_nutation(tt).0 * compute_obliquity(tt).cos();
        let gast = astro::earth_rotation_angle(ut1) - eo;
        let difference = (gast - row["gast"].as_f64().unwrap()).sin().asin().abs().to_degrees() * 3600.0;
        assert!(difference < 0.01, "GAST error {difference}");
        let actual = compute_slow_orientation(tt);
        for i in 0..3 {
            assert!(
                angle(
                    Vector3 {
                        x: actual.0[i][0],
                        y: actual.0[i][1],
                        z: actual.0[i][2]
                    },
                    vector(&row["c2i06a"][i])
                ) < 0.03
            );
        }
    }
}

#[test]
fn measured_range_endpoints_use_the_documented_gregorian_tt_dates() {
    let jd = |s: &str| astro::datetime_to_julian_date(&astro::parse_utc_datetime(s).unwrap());
    assert_eq!(
        astro::PLANET_VALIDATED_INTERVAL.unwrap().start_tt,
        jd("1850-01-01T00:00:00")
    );
    assert_eq!(
        astro::PLANET_VALIDATED_INTERVAL.unwrap().end_tt,
        jd("2030-01-01T00:00:00")
    );
    assert_eq!(
        astro::MOON_VALIDATED_INTERVAL.unwrap().start_tt,
        jd("0000-01-01T00:00:00")
    );
    assert_eq!(
        astro::MOON_VALIDATED_INTERVAL.unwrap().end_tt,
        jd("4000-01-01T00:00:00")
    );
}
