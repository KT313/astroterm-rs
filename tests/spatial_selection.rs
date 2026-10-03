//! Spatial and brightness selection against an independent full scan of the same immutable stored trajectories.
use astroterm::{
    astro::{COMPUTATIONAL_INTERVAL, J2000, JULIAN_YEAR_DAYS, Observer, Vector3, refract_direction},
    canvas::Canvas,
    catalog::{CatalogStar, SpaceMotion, StarId, load_embedded_catalog},
    projection::{ProjectionKind, View, ViewCenter, Viewport, project_sky},
    scene::{RenderOptions, draw_sky_scene},
    sky::{
        FrameTime, ObservedSky, ObservedStar, SimulationState, SkyCatalog, observe_sky, prepare_observation,
        update_simulation,
    },
    timing::StepTimes,
};
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
                    magnitude: (uniform() * 12.0) as f32,
                    spectral_type: *b"G2",
                    color_index: None,
                    has_data: true,
                });
            }
            Arc::new(SkyCatalog::from_owned_catalog(catalog))
        })
        .clone()
}

fn compare(date: f64, view: View, threshold: f64, refraction: bool, latitude: f64, longitude: f64) {
    let time = FrameTime::from_utc(date);
    let mut simulation = SimulationState::exact();
    let mut timing = StepTimes::default();
    update_simulation(&mut simulation, time, &[], &mut timing).unwrap();
    let observer = prepare_observation(&mut simulation, time, Observer { latitude, longitude }).unwrap();
    let mut selected = ObservedSky::new(catalog());
    observe_sky(
        &simulation,
        &observer,
        threshold,
        refraction,
        view.sky_region(),
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
    let a = project_sky(&selected, &view, viewport);
    let b = project_sky(&full, &view, viewport);
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

#[test]
fn boundary_dates_and_extreme_fields_of_view_match_full_scan() {
    for date in [
        COMPUTATIONAL_INTERVAL.start_tt - 1.0,
        COMPUTATIONAL_INTERVAL.start_tt,
        J2000,
        COMPUTATIONAL_INTERVAL.end_tt.next_down(),
        COMPUTATIONAL_INTERVAL.end_tt,
    ] {
        for fov in [1.0, 180.0, 300.0, 359.0, 360.0] {
            for refraction in [false, true] {
                compare(
                    date,
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
                    0.0,
                    0.0,
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn random_regions_epochs_and_thresholds_preserve_exact_visible_sets(
        date in (COMPUTATIONAL_INTERVAL.start_tt-365250.0)..(COMPUTATIONAL_INTERVAL.end_tt+365250.0),
        azimuth in 0.0_f64..std::f64::consts::TAU, tilt in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2,
        fov in 1.0_f64..359.0, threshold in -2.0_f64..14.0, refraction in any::<bool>(),
        latitude in -std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2, longitude in -std::f64::consts::PI..std::f64::consts::PI,
        equidistant in any::<bool>(),
    ) {
        compare(date,View { center:ViewCenter::Facing { azimuth,tilt },fov_degrees:fov,
            projection: if equidistant { ProjectionKind::Equidistant } else { ProjectionKind::Stereographic } },threshold,refraction,latitude,longitude);
    }
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
        for (i, magnitude) in [5_f32.next_down(), 5.0, 5_f32.next_up()].into_iter().enumerate() {
            let mut star = template.clone();
            star.id = StarId(i as u64);
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
        let catalog = Arc::new(SkyCatalog::from_owned_catalog(parsed));
        let time = FrameTime::from_utc(J2000 + years * JULIAN_YEAR_DAYS);
        let mut simulation = SimulationState::exact();
        let mut timing = StepTimes::default();
        update_simulation(&mut simulation, time, &[], &mut timing).unwrap();
        let mut observer = prepare_observation(&mut simulation, time, Observer::default()).unwrap();
        observer.inertial_to_horizon = Matrix3::IDENTITY;
        observer.state.velocity = Vector3::default();
        let mut sky = ObservedSky::new(catalog);
        observe_sky(
            &simulation,
            &observer,
            5.0,
            refraction,
            view.sky_region(),
            &mut sky,
            &mut timing,
        )
        .unwrap();
        let projected = project_sky(&sky, &view, Viewport { height: 81, width: 161 });
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
                let visible = astroterm::projection::CartesianCamera::new(&view)
                    .project(position)
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
