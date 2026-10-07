//! 3D motion, conservative selection, current-magnitude ordering and endpoint independence.
use astroterm::state::{SimulationState};
use astroterm::astro::{Horizontal, J2000, JULIAN_YEAR_DAYS, Matrix3, Observer};
use astroterm::canvas::Canvas;
use astroterm::catalog::{Catalog, CatalogStar, ConstellationFigure, SpaceMotion, StarId, StarNames};
use astroterm::model::{
    ObservedSky, SkyCatalog, ProjectionViewport as Viewport, View, ViewCenter, RenderOptions, FrameTime,
};
use astroterm::projection::project_sky;
use astroterm::scene::draw_sky_scene;
use astroterm::sky::{observe_sky, observe_sky_candidates, prepare_observation, update_solar_system};
use astroterm::timing::StepTimes;
use std::sync::Arc;

fn star(id: u32, mag: f32, h: Horizontal, radial: f64) -> CatalogStar {
    let u = h.to_unit_vector();
    CatalogStar {
        id: StarId(id),
        hr: Some(id),
        name: None,
        designation: None,
        right_ascension: u.y.atan2(u.x),
        declination: u.z.asin(),
        ra_motion: 0.0,
        ra_motion_cos_dec: 0.0,
        dec_motion: 0.0,
        magnitude: f64::from(mag),
        spectral_type: *b"A0",
        color_index: None,
        has_data: true,
        space_motion: Some(SpaceMotion {
            distance_pc: 10.0,
            position: u * 10.0,
            velocity: u * (10.0 * radial),
        }),
    }
}
fn catalog(stars: Vec<CatalogStar>, segments: Vec<[u32; 2]>) -> Arc<SkyCatalog> {
    Arc::new(astroterm::sky::prepare_catalog(&Catalog::new(
        stars,
        StarNames::default(),
        vec![ConstellationFigure {
            abbreviation: "Test",
            segments,
        }],
    )).unwrap().catalog)
}
fn setup(years: f64) -> (SimulationState, astroterm::model::ObserverState) {
    let tt = J2000 + years * JULIAN_YEAR_DAYS;
    let time = FrameTime { utc: tt, ut1: tt, tt };
    let mut simulation = SimulationState::exact();
    update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
    let mut observer = prepare_observation(&mut simulation, time, Observer::default()).unwrap();
    observer.inertial_to_horizon = Matrix3::IDENTITY;
    observer.state.velocity = astroterm::astro::Vector3::default(); // known synthetic directions, independent of sidereal rotation
    (simulation, observer)
}
fn options(threshold: f64) -> RenderOptions {
    RenderOptions {
        unicode: true,
        braille: true,
        color: true,
        constellations: true,
        grid: false,
        magnitude_threshold: threshold,
        label_threshold: 0.25,
        dynamic_names: true,
    }
}
fn horizontal(az: f64, alt: f64) -> Horizontal {
    Horizontal {
        azimuth: az.to_radians(),
        altitude: alt.to_radians(),
    }
}

#[test]
fn threshold_crossing_uses_interval_key_then_current_magnitude() {
    let cat = catalog(vec![star(1, 5.5, horizontal(0.0, 60.0), -0.00005)], vec![]);
    assert!(cat.stars.get(0).brightness_key < 5.0);
    let mut sky = ObservedSky::new(cat);
    for (years, drawn) in [
        (0.0, false),
        (3000.0, false),
        (5000.0, true),
        (15000.0, true),
        (20000.0, false),
    ] {
        let (simulation, observer) = setup(years);
        observe_sky(
            &simulation,
            &observer,
            5.0,
            false,
            astroterm::model::SkyRegion::All,
            &mut sky,
            &mut StepTimes::default(),
        )
        .unwrap();
        let projected_data = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
        let projected = projected_data.view(&sky);
        assert_eq!(!projected.stars.is_empty(), drawn, "year offset {years}");
        if years == 20000.0 {
            assert_eq!(sky.runtime_singular_count, 1);
            assert!(sky.stars.is_empty());
            assert_eq!(sky.corrections.skipped, 1);
            assert_eq!(sky.catalog.stars.motion(0).evaluate(years, 5.5).magnitude, 5.5);
        }
    }
}

#[test]
fn out_of_interval_selection_does_not_use_expired_brightness_keys() {
    let cat = catalog(vec![star(1, 6.0, horizontal(0.0, 60.0), -1.0 / 30000.0)], vec![]);
    assert!(cat.stars.get(0).brightness_key > 4.0);
    let mut sky = ObservedSky::new(cat);
    let (simulation, observer) = setup(24000.0);
    observe_sky_candidates(
        &simulation,
        &observer,
        4.0,
        false,
        Some(&[]),
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    assert_eq!(sky.stars.len(), 1);
    assert!(sky.stars[0].drawable && sky.stars[0].magnitude < 4.0);
}

#[test]
fn visible_drawing_order_is_current_magnitude_then_stable_id() {
    let cat = catalog(
        vec![
            star(1, 5.5, horizontal(0.0, 60.0), -0.00005),
            star(2, 5.0, horizontal(0.0, 60.0), 0.0),
            star(3, 5.0, horizontal(0.0, 60.0), 0.0),
        ],
        vec![],
    );
    let mut sky = ObservedSky::new(cat);
    for (years, expected) in [(0.0, vec![1, 2, 3]), (5000.0, vec![2, 3, 1])] {
        let (simulation, observer) = setup(years);
        observe_sky(
            &simulation,
            &observer,
            6.0,
            false,
            astroterm::model::SkyRegion::All,
            &mut sky,
            &mut StepTimes::default(),
        )
        .unwrap();
        let projected_data = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
        let projected = projected_data.view(&sky);
        assert_eq!(
            projected.stars.iter().map(|p| p.star.id().0).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn refracted_constellation_endpoint_outside_selection_matches_full_observation() {
    let cat = catalog(
        vec![
            star(1, 1.0, horizontal(0.0, 0.1), 0.0),
            star(2, 1.0, horizontal(100.0, 0.1), 0.0),
        ],
        vec![[1, 2], [1, 2]],
    );
    let inside = cat.stars.iter().position(|star| star.id == StarId(1)).unwrap();
    let mut full = ObservedSky::new(cat.clone());
    let mut restricted = ObservedSky::new(cat);
    let (simulation, observer) = setup(0.0);
    observe_sky(
        &simulation,
        &observer,
        5.0,
        true,
        astroterm::model::SkyRegion::All,
        &mut full,
        &mut StepTimes::default(),
    )
    .unwrap();
    observe_sky_candidates(
        &simulation,
        &observer,
        5.0,
        true,
        Some(&[inside, inside]),
        &mut restricted,
        &mut StepTimes::default(),
    )
    .unwrap();
    assert_eq!(restricted.stars.len(), 2);
    for (a, b) in full.stars.iter().zip(&restricted.stars) {
        assert_eq!(a.position, b.position);
    }
    let expected =
        astroterm::astro::refract_direction(restricted.catalog.stars.motion(inside).evaluate(0.0, 5.0).direction);
    let actual = restricted
        .star_views()
        .find(|star| star.id() == StarId(1))
        .unwrap()
        .position;
    assert!((expected - actual).length() < 1e-14);
    let view = View {
        center: ViewCenter::Facing {
            azimuth: 0.0,
            tilt: 0.0,
        },
        fov_degrees: 90.0,
        ..View::default()
    };
    let viewport = Viewport { height: 41, width: 81 };
    let (mut a, mut b) = (Canvas::new(41, 81), Canvas::new(41, 81));
    draw_sky_scene(&mut a, &options(5.0), &project_sky(&full, &view, viewport).view(&full));
    draw_sky_scene(&mut b, &options(5.0), &project_sky(&restricted, &view, viewport).view(&restricted));
    assert_eq!(a.to_lines(), b.to_lines());
}

#[test]
fn singular_trajectories_are_rejected_and_supported_fast_motion_stays_indexed() {
    let mut fast = star(2, 4.0, horizontal(0.0, 60.0), 0.0);
    fast.space_motion = None;
    fast.dec_motion = 0.0001;
    let source = Catalog::new(vec![star(1, 4.0, horizontal(0.0, 60.0), -0.001), fast.clone()], StarNames::default(), vec![]);
    let error = astroterm::sky::prepare_catalog(&source).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert!(error.to_string().contains("Star 1 requires tangential-motion fallback"));
    let cat = catalog(vec![fast], vec![]);
    assert!(cat.star_exceptions.is_empty());
    assert_eq!(cat.singular_count, 0);
    assert!(cat.always_checked().any(|i| cat.stars.get(i).id == StarId(2)));
    let mut sky = ObservedSky::new(cat);
    let (simulation, observer) = setup(1000.0);
    observe_sky(
        &simulation,
        &observer,
        5.0,
        false,
        astroterm::model::SkyRegion::All,
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    let projected_data = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
    let projected = projected_data.view(&sky);
    assert_eq!(projected.catalog_singular_count, 0);
    assert_eq!(projected.runtime_singular_count, 0);
    assert!(sky.stars.iter().all(|star| star.position.x.is_finite()));
}

#[test]
fn normalized_motion_change_stays_within_a_derived_legacy_bound() {
    use astroterm::astro::{
        Equatorial,
        models::stars::{StellarMotion, compute_star_position},
    };
    for dec in [-89.99_f64, -45.0, 0.0, 45.0, 89.99] {
        let direction = Equatorial {
            right_ascension: 1.0,
            declination: dec.to_radians(),
        };
        let rates = Equatorial {
            right_ascension: 0.00001,
            declination: -0.00002,
        };
        let star = StellarMotion::from_angles(direction, rates);
        for years in [-10000.0, -1000.0, -25.0, 0.0, 25.0, 1000.0, 10000.0] {
            let date = J2000 + years * JULIAN_YEAR_DAYS;
            let old = compute_star_position(direction, rates, date).to_unit_vector();
            let new = star.evaluate(years, 5.0).direction;
            let angle = old.cross(new).length().atan2(old.dot(new));
            let legacy_years = (date - J2000) / 365.2425;
            let rate = rates.right_ascension.abs() + rates.declination.abs();
            // Spherical Taylor remainder <= rate²t²/2; normalization doubles chord error, arc <= π/2 chord.
            let bound =
                std::f64::consts::FRAC_PI_2 * (rate * legacy_years).powi(2) + rate * (legacy_years - years).abs();
            assert!(angle <= bound + 1e-12, "dec {dec}, t {years}: {angle} > {bound}");
        }
    }
}

#[test]
fn correction_selection_keeps_faint_endpoints_but_discards_other_rejected_stars() {
    let cat = catalog(
        vec![
            star(1, 1.0, horizontal(0.0, 20.0), 0.0),
            star(2, 5.5, horizontal(100.0, 0.1), -0.00005),
            star(3, 5.5, horizontal(40.0, 30.0), -0.00005),
        ],
        vec![[1, 2]],
    );
    let endpoint = cat.stars.iter().position(|s| s.id == StarId(2)).unwrap();
    let candidates: Vec<_> = cat
        .stars
        .iter()
        .enumerate()
        .filter(|(_, s)| s.id != StarId(2))
        .map(|(i, _)| i)
        .collect();
    let expected = astroterm::astro::refract_direction(cat.stars.motion(endpoint).evaluate(0.0, 5.5).direction);
    let mut sky = ObservedSky::new(cat);
    let (simulation, observer) = setup(0.0);
    observe_sky_candidates(
        &simulation,
        &observer,
        5.0,
        true,
        Some(&candidates),
        &mut sky,
        &mut StepTimes::default(),
    )
    .unwrap();
    assert_eq!(
        (
            sky.corrections.evaluated,
            sky.corrections.skipped,
            sky.corrections.endpoint_only
        ),
        (3, 1, 1)
    );
    assert!(!sky.star_views().any(|s| s.id() == StarId(3)));
    let retained = sky.star_views().find(|s| s.id() == StarId(2)).unwrap();
    assert!(!retained.drawable);
    assert!((retained.position - expected).length() < 1e-14);
}
