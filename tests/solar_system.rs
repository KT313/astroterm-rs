//! Steps 1 and 2 of the rebuilt frame (`simulate_solar_system`, `observe`) against the behaviour the old
//! simulation tests established: sites, Moon phase, refraction once, and the aberration formula.
use astroterm::astro::{J2000, MoonPhase, Observer, Vector3, apply_aberration, name_moon_phase, refract_direction};
use astroterm::model::{Data, View};
use astroterm::observation::observe;
use astroterm::solar_system::simulate_solar_system;

fn angle_arcseconds(a: Vector3, b: Vector3) -> f64 {
    a.cross(b).length().atan2(a.dot(b)).to_degrees() * 3600.0
}

#[test]
fn the_moon_shifts_by_its_parallax_between_sites() {
    let time = astroterm::astro::utc_jd_to_time_ms(J2000).unwrap();
    let system = simulate_solar_system(&Data::default(), time, &mut astroterm::time::StepTimes::default());
    let a = observe(&Observer::default(), false, &View::default(), time, &mut astroterm::time::StepTimes::default());
    let b = observe(&Observer { latitude: 0.7, longitude: 1.1 }, false, &View::default(), time, &mut astroterm::time::StepTimes::default());
    let moon_a = a.apparent_direction(system.moon_position(0, a.observer.position));
    let moon_b = b.apparent_direction(system.moon_position(0, b.observer.position));
    let geocentric = a.to_horizon.apply((system.moons[0].position - system.earth.position).normalized());
    let topocentric = a.to_horizon.apply((system.moon_position(0, a.observer.position) - a.observer.position).normalized());
    assert!(angle_arcseconds(geocentric, topocentric) > 1000.0, "parallax");           // up to about 1°
    assert!(angle_arcseconds(a.to_catalog.apply(moon_a), b.to_catalog.apply(moon_b)) > 1000.0); // two sites see the Moon against different stars
    assert!(angle_arcseconds(moon_a, moon_b) > 3600.0);
    assert_eq!(simulate_solar_system(&Data::default(), time, &mut astroterm::time::StepTimes::default()), system);                 // nothing about the observer changes the simulation
}

#[test]
fn continuous_moon_phase_matches_horizons_and_original_reference_dates() {
    let references: serde_json::Value = serde_json::from_str(include_str!("fixtures/reference/moon_phase.json")).unwrap();
    let bodies = Data::default();
    for row in references["rows"].as_array().unwrap() {
        let tt = row["Date_________JDTT"].as_str().unwrap().parse::<f64>().unwrap();
        let expected = row["Illu%"].as_str().unwrap().parse::<f64>().unwrap() / 100.0;
        let stamp_ms = astroterm::astro::utc_jd_to_time_ms(tt).unwrap();                                  // the fixture is in TT
        let actual = astroterm::solar_system::calculate_solar_system_at_tt(&bodies, stamp_ms, tt, &mut astroterm::time::StepTimes::default()).moon_illumination(0).illuminated_fraction;
        println!("TT {tt}: illuminated fraction {actual:.9}, Horizons {expected:.9}, difference {:.9}", actual - expected);
        assert!((actual - expected).abs() < 0.001);                                       // model-comparison envelope
    }
    for (tt, phase) in [(2451550.1, MoonPhase::New), (2460645.5, MoonPhase::New), (2459242.5, MoonPhase::Full), (2466447.5, MoonPhase::Full)] {
        let illumination = astroterm::solar_system::calculate_solar_system_at_tt(&bodies, astroterm::astro::utc_jd_to_time_ms(tt).unwrap(), tt, &mut astroterm::time::StepTimes::default()).moon_illumination(0);
        assert_eq!(name_moon_phase(illumination), phase, "{tt}");
    }
}

#[test]
fn refraction_is_applied_once_and_the_sun_follows_the_aberration_formula() {
    let time = astroterm::astro::utc_jd_to_time_ms(2459146.0).unwrap();
    let site = Observer { latitude: 0.74, longitude: -1.24 };
    let system = simulate_solar_system(&Data::default(), time, &mut astroterm::time::StepTimes::default());
    let airless = observe(&site, false, &View::default(), time, &mut astroterm::time::StepTimes::default());
    let refracted = observe(&site, true, &View::default(), time, &mut astroterm::time::StepTimes::default());
    let sun = airless.apparent_direction(system.sun);
    let expected = airless.to_horizon.apply(apply_aberration(system.sun - airless.observer.position, airless.observer.velocity));
    assert!((sun - expected).length() < 1e-8);
    assert_eq!(refracted.apparent_direction(system.sun), refract_direction(sun));
    assert_ne!(refract_direction(refract_direction(sun)), refract_direction(sun));        // refraction is not idempotent, so it must be applied exactly once
    let again = observe(&site, true, &View::default(), time, &mut astroterm::time::StepTimes::default());
    assert_eq!(again, refracted);                                                         // the same inputs give the same observation
}

/// Release-mode timing of the two steps; run with `--ignored --nocapture`.
#[test]
#[ignore = "timing only"]
fn time_the_first_two_steps() {
    let bodies = Data::default();
    let site = Observer { latitude: 0.74, longitude: -1.24 };
    let started = std::time::Instant::now();
    let frames = 1000;
    for frame in 0..frames {
        let time = astroterm::astro::utc_jd_to_time_ms(J2000 + f64::from(frame) / 86_400.0).unwrap();
        std::hint::black_box(simulate_solar_system(&bodies, time, &mut astroterm::time::StepTimes::default()));
    }
    let solar = started.elapsed() / frames;
    let started = std::time::Instant::now();
    for frame in 0..frames {
        let time = astroterm::astro::utc_jd_to_time_ms(J2000 + f64::from(frame) / 86_400.0).unwrap();
        std::hint::black_box(observe(&site, true, &View::default(), time, &mut astroterm::time::StepTimes::default()));
    }
    eprintln!("simulate_solar_system {solar:?} per frame; observe {:?} per frame", started.elapsed() / frames);
}
