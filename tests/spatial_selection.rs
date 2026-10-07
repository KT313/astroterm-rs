//! Spatial and brightness selection against an independent full scan of the same immutable stored trajectories.
use astroterm::state::{SimulationState};
use astroterm::astro::{COMPUTATIONAL_INTERVAL, J2000, JULIAN_YEAR_DAYS, Observer, Vector3, refract_direction};
use astroterm::canvas::Canvas;
use astroterm::catalog::{CatalogStar, SpaceMotion, StarId, load_embedded_catalog};
use astroterm::model::{
    ObservedSky, ObserverState, SkyCatalog, ObservedStar, ProjectionKind, ProjectionViewport as Viewport, View,
    ViewCenter, RenderOptions, FrameTime,
};
use astroterm::projection::project_sky;
use astroterm::scene::draw_sky_scene;
use astroterm::sky::{observe_sky, prepare_observation, update_solar_system};
use astroterm::timing::StepTimes;
use proptest::prelude::*;
use std::sync::{Arc, OnceLock};

fn catalog() -> Arc<SkyCatalog> {
    static CATALOG: OnceLock<Arc<SkyCatalog>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let mut catalog = load_embedded_catalog().unwrap();
            let mut seed = 17_u64;
            let mut uniform = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 11) as f64 / (1_u64 << 53) as f64
            };
            for i in 0..4000 {
                let ra = uniform() * std::f64::consts::TAU;
                let dec = (2.0 * uniform() - 1.0).asin();
                let position = astroterm::astro::Equatorial {
                    right_ascension: ra,
                    declination: dec,
                }
                .to_unit_vector();
                let speed = if i % 10 == 0 { 0.001 } else { 1e-8 };
                let velocity = Vector3 {
                    x: (uniform() - 0.5) * speed,
                    y: (uniform() - 0.5) * speed,
                    z: (uniform() - 0.5) * speed,
                };
                catalog.stars.push(CatalogStar {
                    id: StarId(10000 + i),
                    space_motion: Some(SpaceMotion {
                        distance_pc: 1.0,
                        position,
                        velocity,
                    }),
                    hr: None,
                    name: None,
                    designation: None,
                    right_ascension: ra,
                    declination: dec,
                    ra_motion: 0.0,
                    ra_motion_cos_dec: 0.0,
                    dec_motion: 0.0,
                    magnitude: uniform() * 12.0,
                    spectral_type: *b"G2",
                    color_index: None,
                    has_data: true,
                });
            }
            Arc::new(astroterm::sky::prepare_owned_catalog(catalog).unwrap().catalog)
        })
        .clone()
}

fn prepare_case(date: f64, latitude: f64, longitude: f64) -> (SimulationState, ObserverState) {
    let time = FrameTime::from_utc(date);
    let mut simulation = SimulationState::exact();
    update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
    let observer = prepare_observation(&mut simulation, time, Observer { latitude, longitude }).unwrap();
    (simulation, observer)
}

fn compare(date: f64, view: View, threshold: f64, refraction: bool, latitude: f64, longitude: f64) {
    let (simulation, observer) = prepare_case(date, latitude, longitude);
    compare_prepared(&simulation, &observer, view, threshold, refraction);
}

fn compare_prepared(
    simulation: &SimulationState,
    observer: &ObserverState,
    view: View,
    threshold: f64,
    refraction: bool,
) {
    let time = observer.time;
    let mut timing = StepTimes::default();
    let mut selected = ObservedSky::new(catalog());
    observe_sky(
        simulation,
        observer,
        threshold,
        refraction,
        astroterm::projection::select_view_region(&view),
        &mut selected,
        &mut timing,
    )
    .unwrap();

    // scan every star without consulting keys, bounds, the grid or the always-checked list
    let mut full = selected.clone();
    let years = (time.tt - J2000) / JULIAN_YEAR_DAYS;
    full.stars = full
        .catalog
        .stars
        .iter()
        .enumerate()
        .map(|(i, star)| {
            let sample = star.motion.evaluate(years, star.magnitude);
            let aberrated =
                (sample.direction.normalized() + observer.state.velocity * (1.0 / 173.144632674240)).normalized();
            let position = observer.inertial_to_horizon.apply(aberrated);
            let position = if refraction {
                refract_direction(position)
            } else {
                position
            };
            let mut observed = ObservedStar::from_star(&star, i, position);
            observed.magnitude = sample.magnitude;
            observed.drawable = sample.magnitude <= threshold;
            observed
        })
        .collect();
    let viewport = Viewport { height: 81, width: 161 };
    let a_data = project_sky(&selected, &view, viewport);
    let a = a_data.view(&selected);
    let b_data = project_sky(&full, &view, viewport);
    let b = b_data.view(&full);
    assert_eq!(
        a.stars.iter().map(|s| (s.star.id(), s.cell)).collect::<Vec<_>>(),
        b.stars.iter().map(|s| (s.star.id(), s.cell)).collect::<Vec<_>>()
    );
    assert_eq!(a.constellations, b.constellations);
    let options = RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: threshold,
        label_threshold: 0.25,
        dynamic_names: true,
    };
    let mut ca = Canvas::new(81, 161);
    let mut cb = Canvas::new(81, 161);
    draw_sky_scene(&mut ca, &options, &a);
    draw_sky_scene(&mut cb, &options, &b);
    assert_eq!(ca, cb);
    if !COMPUTATIONAL_INTERVAL.contains(time.tt) {
        assert!(selected.selection.brute_force);
        assert_eq!(selected.corrections.evaluated, full.catalog.stars.len());
    }
}

fn check_boundary_date(date: f64) {
    let (simulation, observer) = prepare_case(date, 0.0, 0.0);
    for fov in [1.0, 180.0, 300.0, 359.0, 360.0] {
        for refraction in [false, true] {
            compare_prepared(
                &simulation,
                &observer,
                View {
                    fov_degrees: fov,
                    projection: ProjectionKind::Equidistant,
                    center: ViewCenter::Facing {
                        azimuth: 0.0,
                        tilt: 0.0,
                    },
                },
                5.0,
                refraction,
            );
        }
    }
}

#[test]
fn views_before_interval_match_full_scan() {
    check_boundary_date(COMPUTATIONAL_INTERVAL.start_tt - 1.0);
}

#[test]
fn views_at_interval_start_match_full_scan() {
    check_boundary_date(COMPUTATIONAL_INTERVAL.start_tt);
}

#[test]
fn views_at_j2000_match_full_scan() {
    check_boundary_date(J2000);
}

#[test]
fn views_just_before_interval_end_match_full_scan() {
    check_boundary_date(COMPUTATIONAL_INTERVAL.end_tt.next_down());
}

#[test]
fn views_at_interval_end_match_full_scan() {
    check_boundary_date(COMPUTATIONAL_INTERVAL.end_tt);
}

// Four independent property tests retain 128 cases in total and guarantee coverage of both projections
// with and without refraction. Each runner remains sequential and retains proptest shrinking/replay.
macro_rules! test_random_regions {
    ($name:ident, $projection:expr, $refraction:expr) => {
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(32))]
            #[test]
            fn $name(
                date in (COMPUTATIONAL_INTERVAL.start_tt-365250.0)..(COMPUTATIONAL_INTERVAL.end_tt+365250.0),
                azimuth in 0.0_f64..std::f64::consts::TAU,
                tilt in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2,
                fov in 1.0_f64..359.0, threshold in -2.0_f64..14.0,
                latitude in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2,
                longitude in -std::f64::consts::PI..std::f64::consts::PI,
            ) {
                compare(date, View {
                    center: ViewCenter::Facing { azimuth, tilt },
                    fov_degrees: fov,
                    projection: $projection,
                }, threshold, $refraction, latitude, longitude);
            }
        }
    };
}

test_random_regions!(
    random_stereographic_airless_matches_full_scan,
    ProjectionKind::Stereographic,
    false
);
test_random_regions!(
    random_stereographic_refracted_matches_full_scan,
    ProjectionKind::Stereographic,
    true
);
test_random_regions!(
    random_equidistant_airless_matches_full_scan,
    ProjectionKind::Equidistant,
    false
);
test_random_regions!(
    random_equidistant_refracted_matches_full_scan,
    ProjectionKind::Equidistant,
    true
);

// Preserve the concrete failure recorded in spatial_selection.proptest-regressions even when the
// randomized strategy changes its input layout (the separate projection/refraction tests above).
#[test]
fn saved_stereographic_selection_regression_matches_full_scan() {
    compare(
        0.0,
        View {
            center: ViewCenter::Facing {
                azimuth: 0.0,
                tilt: 0.0,
            },
            fov_degrees: 170.70431445631192,
            projection: ProjectionKind::Stereographic,
        },
        5.381495729624377,
        false,
        0.0,
        0.0,
    );
}

#[test]
fn seam_threshold_horizon_fast_mover_and_view_edge_cases_are_not_culled() {
    use astroterm::astro::{Horizontal, Matrix3};
    for (direction, rate, years, view, refraction) in [
        (
            Vector3 { x: 1.0, y: 1.0, z: 1.0 }.normalized(),
            Vector3::default(),
            0.0,
            View::default(),
            false,
        ),
        (
            Vector3 { x: 0.0, y: 0.0, z: 1.0 },
            Vector3::default(),
            0.0,
            View::default(),
            false,
        ),
        (
            Horizontal {
                azimuth: 0.0,
                altitude: -1_f64.to_radians(),
            }
            .to_unit_vector(),
            Vector3::default(),
            0.0,
            View {
                center: ViewCenter::Facing {
                    azimuth: 0.0,
                    tilt: 89.5_f64.to_radians(),
                },
                ..View::default()
            },
            true,
        ),
        (
            Horizontal {
                azimuth: 0.0,
                altitude: 0.001_f64.to_radians(),
            }
            .to_unit_vector(),
            Vector3::default(),
            0.0,
            View::default(),
            false,
        ),
        (
            Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            Vector3 {
                x: 0.0,
                y: 0.0,
                z: 0.001,
            },
            10000.0,
            View::default(),
            false,
        ),
        (
            Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            Vector3 {
                x: 0.0,
                y: 0.0,
                z: -0.001,
            },
            -9900.0,
            View::default(),
            false,
        ),
    ] {
        let mut parsed = load_embedded_catalog().unwrap();
        let template = parsed.stars[0].clone();
        parsed.stars.clear();
        parsed.constellations.clear();
        parsed.hr_representatives.clear();
        for (i, magnitude) in [4.999, 5.0, 5.001].into_iter().enumerate() {
            let mut star = template.clone();
            star.id = StarId(i as u32);
            star.has_data = true;
            star.magnitude = magnitude - if years != 0.0 { 10.0 } else { 0.0 };
            star.right_ascension = direction.y.atan2(direction.x);
            star.declination = direction.z.asin();
            star.space_motion = Some(SpaceMotion {
                distance_pc: 1.0,
                position: direction,
                velocity: rate,
            });
            parsed.stars.push(star);
        }
        let catalog = Arc::new(astroterm::sky::prepare_owned_catalog(parsed).unwrap().catalog);
        let time = FrameTime::from_utc(J2000 + years * JULIAN_YEAR_DAYS);
        let mut simulation = SimulationState::exact();
        let mut timing = StepTimes::default();
        update_solar_system(&mut simulation, time, &[], &mut timing).unwrap();
        let mut observer = prepare_observation(&mut simulation, time, Observer::default()).unwrap();
        observer.inertial_to_horizon = Matrix3::IDENTITY;
        observer.state.velocity = Vector3::default();
        let mut sky = ObservedSky::new(catalog);
        observe_sky(
            &simulation,
            &observer,
            5.0,
            refraction,
            astroterm::projection::select_view_region(&view),
            &mut sky,
            &mut timing,
        )
        .unwrap();
        let projected_data = project_sky(&sky, &view, Viewport { height: 81, width: 161 });
        let projected = projected_data.view(&sky);
        let expected: Vec<_> = sky
            .catalog
            .stars
            .iter()
            .filter_map(|s| {
                let sample = s.motion.evaluate(years, s.magnitude);
                let position = if refraction {
                    refract_direction(sample.direction)
                } else {
                    sample.direction
                };
                let visible = astroterm::projection::project_camera(astroterm::projection::prepare_camera(&view), position)
                    .is_some_and(|p| p.is_visible());
                (sample.magnitude <= 5.0 && visible).then_some(s.id)
            })
            .collect();
        assert!(!expected.is_empty());
        let mut actual: Vec<_> = projected.stars.iter().map(|s| s.star.id()).collect();
        actual.sort_unstable();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(actual, expected);
        if years == 0.0 {
            assert_eq!(actual, vec![StarId(0), StarId(1)]);
        }
    }
}
