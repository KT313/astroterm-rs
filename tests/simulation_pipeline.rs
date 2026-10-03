//! Separation, scheduling, and interpolation accuracy of the four-stage pipeline.
use astroterm::astro::models::{BodyId, BodyState};
use astroterm::astro::{COMPUTATIONAL_INTERVAL, Horizontal, J2000, Matrix3, Observer, Vector3};
use astroterm::canvas::Canvas;
use astroterm::catalog::load_embedded_catalog;
use astroterm::projection::{View, Viewport, project_sky};
use astroterm::scene::{RenderOptions, draw_sky_scene};
use astroterm::sky::{
    FrameTime, ModelFamily, ObservedSky, ObserverState, SimulationState, SkyCatalog, StateRequest, observe_sky,
    prepare_light_time_samples, prepare_observation, prepare_observer, update_simulation,
};
use astroterm::timing::StepTimes;
use std::sync::Arc;

fn angle(a: Vector3, b: Vector3) -> f64 {
    a.cross(b).length().atan2(a.dot(b)).to_degrees() * 3600.0
}
fn frame(tt: f64) -> FrameTime {
    FrameTime { utc: tt, ut1: tt, tt }
}
fn update(state: &mut SimulationState, tt: f64) {
    update_simulation(state, frame(tt), &[], &mut StepTimes::default()).unwrap();
}
fn catalog() -> Arc<SkyCatalog> {
    Arc::new(SkyCatalog::from_catalog(&load_embedded_catalog().unwrap()))
}
fn observe(state: &SimulationState, time: f64, site: Observer, catalog: Arc<SkyCatalog>) -> ObservedSky {
    let mut prepared = state.clone();
    let observer = prepare_observation(&mut prepared, frame(time), site).unwrap();
    let mut sky = ObservedSky::new(catalog);
    observe_sky(
        &prepared,
        &observer,
        5.0,
        false,
        astroterm::sky::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    sky
}
fn options() -> RenderOptions {
    RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: 5.0,
        label_threshold: 0.25,
        dynamic_names: true,
    }
}

#[test]
fn observation_and_projection_report_independent_ordered_passes() {
    let mut simulation = SimulationState::default();
    update(&mut simulation, J2000);
    let observer = prepare_observation(&mut simulation, frame(J2000), Observer::default()).unwrap();
    let mut sky = ObservedSky::new(catalog());
    let mut times = StepTimes::default();
    times.begin_frame();
    times.measure_steps("Observation", |times| {
        observe_sky(
            &simulation,
            &observer,
            5.0,
            true,
            astroterm::sky::SkyRegion::All,
            &mut sky,
            times,
        )
        .unwrap();
    });
    let unchanged = sky.clone();
    times.measure_steps("Projection", |times| {
        astroterm::projection::project_sky_with_times(
            &sky,
            &View::default(),
            Viewport { height: 41, width: 81 },
            times,
        );
    });
    assert_eq!(sky, unchanged);
    let recorded: Vec<_> = times.steps().iter().map(|step| (step.name, step.depth)).collect();
    assert_eq!(
        recorded,
        [
            ("Observation", 0),
            ("Region filtering", 1),
            ("Brightness bounds", 1),
            ("Body sampling", 1),
            ("Candidate validation", 1),
            ("Constellation endpoints", 1),
            ("Stellar motion", 1),
            ("Current brightness", 1),
            ("Correction selection", 1),
            ("Observer subtraction", 1),
            ("Moon illumination", 1),
            ("Aberration", 1),
            ("Horizon rotation", 1),
            ("Refraction", 1),
            ("Projection", 0),
            ("Star projection", 1),
            ("Star draw order", 1),
            ("Body projection", 1),
            ("Constellation projection", 1),
            ("Horizon projection", 1),
        ]
    );
    assert!(times.steps().iter().all(|step| step.average_seconds.is_finite()));
}

#[test]
fn immutable_simulation_supports_multiple_sites_and_observed_sky_multiple_views() {
    let mut simulation = SimulationState::default();
    update(&mut simulation, J2000);
    let original = simulation.clone();
    let catalog = catalog();
    let a = observe(&simulation, J2000, Observer::default(), catalog.clone());
    let b = observe(
        &simulation,
        J2000,
        Observer {
            latitude: 0.7,
            longitude: 1.1,
        },
        catalog,
    );
    assert!(angle(a.moon.position, b.moon.position) > 3600.0);
    assert_eq!(simulation, original);
    let untouched = a.clone();
    let mut view = View::default();
    let viewport = Viewport { height: 41, width: 81 };
    let projected_a = project_sky(&a, &view, viewport);
    view.pan(0.7, -0.6);
    view.zoom(2.0);
    let projected_b = project_sky(&a, &view, viewport);
    let mut canvas_a = Canvas::new(41, 81);
    let mut canvas_b = Canvas::new(41, 81);
    draw_sky_scene(&mut canvas_a, &options(), &projected_a);
    draw_sky_scene(&mut canvas_b, &options(), &projected_b);
    assert_ne!(canvas_a.to_lines(), canvas_b.to_lines());
    assert_eq!(a, untouched);
    update(&mut simulation, J2000); // paused pan/zoom/reset/resize cannot invalidate anything
    assert_eq!(simulation.refresh_counts, original.refresh_counts);
}

#[test]
fn frames_between_ticks_follow_exact_earth_spin() {
    let mut cached = SimulationState::default();
    update(&mut cached, J2000);
    let original = cached.refresh_counts;
    let cat = catalog();
    let first = observe(&cached, J2000, Observer::default(), cat.clone());
    for seconds in [1.0, 3.0, 6.0, 10.0] {
        let tt = J2000 + seconds / 86400.0;
        update(&mut cached, tt);
        let mut direct = SimulationState::exact();
        update(&mut direct, tt);
        let actual = observe(&cached, tt, Observer::default(), cat.clone());
        let expected = observe(&direct, tt, Observer::default(), cat.clone());
        for (a, b) in actual.stars.iter().zip(&expected.stars) {
            assert!(angle(a.position, b.position) < 0.2);
        }
        assert!(angle(first.stars[0].position, actual.stars[0].position) > seconds);
        assert!(angle(actual.moon.position, expected.moon.position) < 1.0);
    }
    assert_eq!(cached.refresh_counts, original);
}

#[test]
fn cadence_tracks_forward_reverse_and_fast_playback_with_no_refresh_jump() {
    let cat = catalog();
    for speed in [1.0, 1000.0, 100000.0, -1.0, -1000.0, -100000.0] {
        let mut cached = SimulationState::default();
        let mut direct = SimulationState::exact();
        for frame in 0..80 {
            let tt = 2460676.5 + speed * frame as f64 / (24.0 * 86400.0);
            update(&mut cached, tt);
            update(&mut direct, tt);
            let actual = observe(
                &cached,
                tt,
                Observer {
                    latitude: 0.5,
                    longitude: -1.2,
                },
                cat.clone(),
            );
            let expected = observe(
                &direct,
                tt,
                Observer {
                    latitude: 0.5,
                    longitude: -1.2,
                },
                cat.clone(),
            );
            for (a, b) in actual.planets.iter().zip(&expected.planets) {
                assert!(angle(a.position, b.position) < 1.0);
            }
            assert!(angle(actual.moon.position, expected.moon.position) < 1.0);
        }
        println!("speed {speed}x, 80 frames: {:?}", cached.refresh_counts);
    }
    // Compare the expired extrapolation's limiting value with a fresh direct state, excluding real motion.
    let mut cached = SimulationState::default();
    update(&mut cached, J2000);
    for delta in [-11.999, 11.999] {
        let tt = J2000 + delta / 86400.0;
        let old = cached.evaluate_body(BodyId::Moon, tt).unwrap();
        let mut direct = SimulationState::exact();
        update(&mut direct, tt);
        let earth = direct.evaluate_body(BodyId::Earth, tt).unwrap().position;
        let fresh = direct.evaluate_body(BodyId::Moon, tt).unwrap();
        assert!(angle(old.position - earth, fresh.position - earth) < 0.4);
    }
}

#[test]
fn emissions_get_separate_bounded_samples_and_same_epoch_parents() {
    let mut state = SimulationState::default();
    let frame = frame(J2000);
    let emission = J2000 - 4.0 / 24.0;
    let request = StateRequest {
        body: BodyId::Neptune,
        tt: emission,
    };
    update_simulation(&mut state, frame, &[request], &mut StepTimes::default()).unwrap();
    let counts = state.refresh_counts;
    update_simulation(&mut state, frame, &[request], &mut StepTimes::default()).unwrap();
    assert_eq!(counts, state.refresh_counts);
    for delta in [-29.99, 0.0, 29.99] {
        let tt = emission + delta / 86400.0;
        let actual = state.evaluate_body(BodyId::Neptune, tt).unwrap();
        let mut direct = SimulationState::exact();
        update(&mut direct, tt);
        let earth = state.evaluate_body(BodyId::Earth, frame.tt).unwrap().position;
        assert!(
            angle(
                actual.position - earth,
                direct.evaluate_body(BodyId::Neptune, tt).unwrap().position - earth
            ) < 0.3
        );
    }
    let planetary_requests: Vec<_> = BodyId::PLANETS
        .into_iter()
        .enumerate()
        .map(|(index, body)| StateRequest {
            body,
            tt: J2000 - (index + 1) as f64 / 48.0,
        })
        .collect();
    update_simulation(&mut state, frame, &planetary_requests, &mut StepTimes::default()).unwrap();
    for request in &planetary_requests {
        assert!(state.evaluate_body(request.body, request.tt).is_ok());
    }
    let lunar_request = StateRequest {
        body: BodyId::Moon,
        tt: emission,
    };
    update_simulation(&mut state, frame, &[lunar_request], &mut StepTimes::default()).unwrap();
    let mut direct = SimulationState::exact();
    update(&mut direct, emission);
    assert_eq!(
        state.evaluate_body(BodyId::Moon, emission).unwrap(),
        direct.evaluate_body(BodyId::Moon, emission).unwrap()
    );
    assert!(state.evaluate_body(BodyId::Moon, emission - 1.0).is_err());
    let excessive = [
        lunar_request,
        StateRequest {
            body: BodyId::Moon,
            tt: emission - 1.0,
        },
    ];
    assert!(update_simulation(&mut state, frame, &excessive, &mut StepTimes::default()).is_err());
}

#[test]
fn model_invalidation_is_limited_to_dependents() {
    let mut state = SimulationState::default();
    update(&mut state, J2000);
    let planets: Vec<_> = BodyId::PLANETS.map(|id| state.evaluate_body(id, J2000).unwrap()).into();
    let old = state.refresh_counts;
    state.set_model_version(ModelFamily::Moon, 1);
    update(&mut state, J2000);
    assert_eq!(state.refresh_counts.planets, old.planets);
    assert_eq!(state.refresh_counts.orientation, old.orientation);
    assert_eq!(state.refresh_counts.moon, old.moon + 1);
    assert_eq!(
        planets,
        BodyId::PLANETS.map(|id| state.evaluate_body(id, J2000).unwrap())
    );
    state.set_model_version(ModelFamily::Orientation, 1);
    update(&mut state, J2000);
    assert_eq!(state.refresh_counts.planets, old.planets);
    assert_eq!(state.refresh_counts.moon, old.moon + 2);
}

#[test]
fn synthetic_anchor_composes_translation_tilt_spin_and_site_velocity() {
    let time = frame(J2000);
    let site = Observer::default();
    let anchor = BodyState {
        position: Vector3 { x: 5.0, y: 2.0, z: 1.0 },
        velocity: Vector3 { x: 0.1, y: 0.2, z: 0.3 },
    };
    let tilt = Matrix3([[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]]);
    let rotation = Matrix3::rotate_z(0.7).compose(tilt);
    let local = BodyState {
        position: Vector3 {
            x: 0.001,
            y: 0.0,
            z: 0.0,
        },
        velocity: Vector3 {
            x: 0.0,
            y: 0.0001,
            z: 0.0,
        },
    };
    let observer = ObserverState::from_anchor_state(time, site, anchor, rotation, local, false);
    assert_eq!(
        observer.state.position,
        anchor.position + rotation.transpose().apply(local.position)
    );
    assert_eq!(
        observer.state.velocity,
        anchor.velocity + rotation.transpose().apply(local.velocity)
    );
    let target = observer.state.position + rotation.transpose().apply(Vector3 { x: 1.0, y: 0.0, z: 0.0 });
    let direction = Horizontal::from_vector(observer.inertial_to_horizon.apply(target - observer.state.position));
    assert!((direction.altitude - std::f64::consts::FRAC_PI_2).abs() < 1e-7);
    let mut simulation = SimulationState::exact();
    update(&mut simulation, time.tt);
    let mut sky = ObservedSky::new(catalog());
    observe_sky(
        &simulation,
        &observer,
        -100.0,
        true,
        astroterm::sky::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    let sun = simulation.evaluate_body(BodyId::Sun, time.tt).unwrap().position;
    let expected_sun = Horizontal::from_vector(
        observer.inertial_to_horizon.apply(
            ((sun - observer.state.position).normalized() + observer.state.velocity * (1.0 / 173.144632674240))
                .normalized(),
        ),
    );
    assert!(angle(sky.sun().position, expected_sun.to_unit_vector()) < 1e-8); // synthetic airless anchor ignores the requested refraction
    assert_ne!(
        rotation.transpose().apply(local.position),
        Matrix3::rotate_z(0.7).transpose().apply(local.position)
    );
}

#[test]
fn unsupported_interval_uses_direct_samples_and_nonfinite_times_fail() {
    let mut state = SimulationState::default();
    let tt = COMPUTATIONAL_INTERVAL.end_tt + 1.0;
    update(&mut state, tt);
    assert!(state.evaluate_body(BodyId::Earth, tt + 1e-5).is_err());
    update(&mut state, COMPUTATIONAL_INTERVAL.end_tt - 1e-5);
    assert!(
        state
            .evaluate_body(BodyId::Earth, COMPUTATIONAL_INTERVAL.end_tt)
            .is_err()
    );
    assert!(update_simulation(&mut state, frame(f64::NAN), &[], &mut StepTimes::default()).is_err());
}

/// Broad deterministic sampling, including dense contemporary lunar cycles and close approaches. This qualifies
/// cache error relative to these exact inherited models, not their astronomical accuracy versus nature.
#[test]
#[ignore = "release cadence qualification sweep"]
fn qualify_cache_intervals_across_the_computational_interval() {
    let mut maxima = [0.0_f64; 4]; // planetary sky, lunar sky, slow orientation, velocity/c
    let mut worst = [0.0; 4];
    let mut absolute = [0.0_f64; 4];
    for index in 0..12000 {
        let epoch = if index < 10000 {
            COMPUTATIONAL_INTERVAL.start_tt
                + 1.0
                + (COMPUTATIONAL_INTERVAL.end_tt - COMPUTATIONAL_INTERVAL.start_tt - 2.0) * index as f64 / 9999.0
        } else {
            J2000 + (index - 10000) as f64 * 0.731
        };
        for sign in [-1.0, 1.0] {
            for (family, seconds) in [(0, 29.99), (1, 11.99), (2, 59.99)] {
                let mut cached = SimulationState::default();
                update(&mut cached, epoch);
                let tt = epoch + sign * seconds / 86400.0;
                if family == 2 {
                    update(&mut cached, tt);
                } // bodies refresh, orientation remains held
                let mut direct = SimulationState::exact();
                update(&mut direct, tt);
                let expected_earth = direct.evaluate_body(BodyId::Earth, tt).unwrap();
                if family == 0 {
                    let actual_earth = cached.evaluate_body(BodyId::Earth, tt).unwrap();
                    for id in BodyId::PLANETS {
                        let a = cached.evaluate_body(id, tt).unwrap();
                        let b = direct.evaluate_body(id, tt).unwrap();
                        absolute[0] = absolute[0].max((a.position - b.position).length());
                        absolute[1] = absolute[1].max((a.velocity - b.velocity).length());
                        let error = if id == BodyId::Earth {
                            0.0
                        } else {
                            angle(a.position - actual_earth.position, b.position - expected_earth.position)
                        };
                        if error > maxima[0] {
                            maxima[0] = error;
                            worst[0] = epoch;
                        }
                        let velocity_error = (a.velocity - b.velocity).length() / 173.144632674240 * 206264.806247;
                        if velocity_error > maxima[3] {
                            maxima[3] = velocity_error;
                            worst[3] = epoch;
                        }
                    }
                } else if family == 1 {
                    let a = cached.evaluate_body(BodyId::Moon, tt).unwrap().position
                        - cached.evaluate_body(BodyId::Earth, tt).unwrap().position;
                    let b = direct.evaluate_body(BodyId::Moon, tt).unwrap().position - expected_earth.position;
                    absolute[2] = absolute[2].max((a - b).length());
                    let av = cached.evaluate_body(BodyId::Moon, tt).unwrap().velocity
                        - cached.evaluate_body(BodyId::Earth, tt).unwrap().velocity;
                    let bv = direct.evaluate_body(BodyId::Moon, tt).unwrap().velocity - expected_earth.velocity;
                    absolute[3] = absolute[3].max((av - bv).length());
                    let error = angle(a, b);
                    if error > maxima[1] {
                        maxima[1] = error;
                        worst[1] = epoch;
                    }
                } else {
                    let delta = cached
                        .evaluate_orientation(tt)
                        .unwrap()
                        .compose(direct.evaluate_orientation(tt).unwrap().transpose());
                    let sine = Vector3 {
                        x: delta.0[2][1] - delta.0[1][2],
                        y: delta.0[0][2] - delta.0[2][0],
                        z: delta.0[1][0] - delta.0[0][1],
                    }
                    .length()
                        / 2.0;
                    let error = sine.asin().to_degrees() * 3600.0;
                    if error > maxima[2] {
                        maxima[2] = error;
                        worst[2] = epoch;
                    }
                }
            }
        }
    }
    println!("maximum cache errors (arcsec): {maxima:?}; worst sample epochs TT: {worst:?}");
    println!("absolute planet/lunar-relative position AU and velocity AU/day maxima: {absolute:?}");
    use astroterm::sky::simulation::{MOON_LIMITS, PLANET_LIMITS};
    for (actual, budget) in absolute.into_iter().zip([
        PLANET_LIMITS.position_au,
        PLANET_LIMITS.velocity_au_day,
        MOON_LIMITS.position_au,
        MOON_LIMITS.velocity_au_day,
    ]) {
        assert!(actual < budget, "absolute error {actual} >= {budget}");
    }
    for (actual, budget) in maxima.into_iter().zip([0.3, 0.4, 0.2, 0.1]) {
        assert!(actual < budget, "{actual} >= {budget}");
    }
}

#[test]
fn continuous_moon_phase_matches_horizons_and_original_reference_dates() {
    let references: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reference/moon_phase.json")).unwrap();
    let cat = catalog();
    for row in references["rows"].as_array().unwrap() {
        let tt = row["Date_________JDTT"].as_str().unwrap().parse::<f64>().unwrap();
        let expected = row["Illu%"].as_str().unwrap().parse::<f64>().unwrap() / 100.0;
        let mut state = SimulationState::exact();
        update(&mut state, tt);
        let mut observer = prepare_observer(&state, frame(tt), Observer::default()).unwrap();
        observer.state = state.evaluate_body(BodyId::Earth, tt).unwrap(); // Horizons fixture is geocentric
        prepare_light_time_samples(&mut state, &mut observer, &mut StepTimes::default()).unwrap();
        let mut sky = ObservedSky::new(cat.clone());
        observe_sky(
            &state,
            &observer,
            5.0,
            false,
            astroterm::sky::SkyRegion::All,
            &mut sky,
            &mut StepTimes::default(),
        )
        .unwrap();
        let actual = sky.moon.illumination.illuminated_fraction;
        println!(
            "TT {tt}: illuminated fraction {actual:.9}, Horizons {expected:.9}, difference {:.9}",
            actual - expected
        );
        assert!((actual - expected).abs() < 0.001); // model-comparison envelope, not interpolation budget
    }
    for (tt, phase) in [
        (2451550.1, astroterm::astro::MoonPhase::New),
        (2460645.5, astroterm::astro::MoonPhase::New),
        (2459242.5, astroterm::astro::MoonPhase::Full),
        (2466447.5, astroterm::astro::MoonPhase::Full),
    ] {
        let mut state = SimulationState::exact();
        update(&mut state, tt);
        assert_eq!(
            observe(&state, tt, Observer::default(), cat.clone()).moon.phase,
            phase,
            "{tt}"
        );
    }
}

#[test]
fn refraction_is_applied_once_in_observation_and_resets_for_each_frame() {
    let mut state = SimulationState::default();
    update(&mut state, J2000);
    let observer = prepare_observer(&state, frame(J2000), Observer::default()).unwrap();
    let mut sky = ObservedSky::new(catalog());
    observe_sky(
        &state,
        &observer,
        5.0,
        false,
        astroterm::sky::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    let raw = sky.moon.position;
    observe_sky(
        &state,
        &observer,
        5.0,
        true,
        astroterm::sky::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    assert_eq!(sky.moon.position, astroterm::astro::refract_direction(raw));
    let once = sky.clone();
    astroterm::sky::refract_sky_positions(&mut sky);
    assert_eq!(sky, once);
    observe_sky(
        &state,
        &observer,
        5.0,
        true,
        astroterm::sky::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    assert_eq!(sky, once);
}
