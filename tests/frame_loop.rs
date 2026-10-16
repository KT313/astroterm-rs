//! The seven steps of the frame loop run headlessly, end to end, on the embedded catalog: the smoke checks the test
//! inventory assigned to chunk 13 (every view and size renders, dynamic names in a zoomed view, Polaris lit).
use astroterm::formulas::Observer;
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{ProjectionKind, RenderOptions, Data, View, ViewCenter};
use astroterm::observation::{Observation, observe};
use astroterm::points::{Drawn, draw_points, draw_panel_and_notices, star_label};
use astroterm::present::{Presented, Terminal, present};
use astroterm::progress::NoProgress;
use astroterm::projection::{Frame, prepare_camera, project_camera, project_stars_to_screen, project_to_cell};
use astroterm::regions::refresh_regions;
use astroterm::selection::select_regions;
use astroterm::sky::{build_sky_tables, prepare_owned_catalog};
use astroterm::solar_system::simulate_solar_system;
use astroterm::time::StepTimes;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;

const TOKYO: Observer = Observer { latitude: 0.6228, longitude: 2.4382 };                   // 35.69° N, 139.69° E
const UTC_2025_03_01_11H: f64 = 2460735.958333333;

fn tables() -> Data {
    let prepared = prepare_owned_catalog(load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap();
    build_sky_tables(&prepared, &mut astroterm::progress::Report::live(&mut NoProgress(std::io::sink()), &mut StepTimes::default())).unwrap()
}

fn settings() -> RenderOptions {
    RenderOptions { unicode: true, braille: false, color: true, constellations: true, grid: false, magnitude_threshold: 6.0, dynamic_names: true }
}

/// One frame through steps 1 to 6 into `frame`; returns the observation and what step 6 drew.
fn run_frame(data: &mut Data, frame: &mut Frame, view: &View, settings: &RenderOptions) -> (Observation, Drawn) {
    let time = astroterm::formulas::utc_jd_to_time_ms(UTC_2025_03_01_11H).unwrap();
    let solar_system = simulate_solar_system(&data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons, time, &mut astroterm::time::StepTimes::default());                                  // 1
    let observation = observe(&TOKYO, true, view, time, astroterm::formulas::evaluate_earth_state(time), &mut astroterm::time::StepTimes::default());                                              // 2
    let selected = select_regions(&data.star_regions, &observation.patch, observation.time_ms, &mut astroterm::time::StepTimes::default());                                       // 3
    refresh_regions(
        &mut data.star_regions, &data.constellation_stars,
        &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
        &mut data.star_opacities, &mut data.tile_bytes,
        &selected.regions, time, view.fov_degrees, settings.magnitude_threshold,
        &mut astroterm::time::StepTimes::default(),
    );                                       // 4
    let mut projection = astroterm::projection::ProjectionExecutor::default();
    let projected_stars = project_stars_to_screen(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, view, &mut projection, &mut astroterm::time::StepTimes::default());                            // 5
    frame.insert_projected_stars(projected_stars);
    let drawn = draw_points(
        frame, &solar_system,
        &data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons,
        &data.constellation_stars, &data.constellation_simulation, &data.constellation_figures,
        &data.star_regions, &data.names, &data.star_opacities, &data.clipping,
        &selected.regions, None,
        &observation, view, settings, &mut astroterm::time::StepTimes::default(),
    );                            // 6
    draw_panel_and_notices(frame, &[], &drawn.notices);
    (observation, drawn)
}

#[test]
fn every_view_renders_at_every_size_including_degenerate_frames() {
    let mut data = tables();
    let views = [
        View { center: ViewCenter::Zenith, projection: ProjectionKind::Stereographic, fov_degrees: 180.0 },
        View { center: ViewCenter::Zenith, projection: ProjectionKind::Equidistant, fov_degrees: 360.0 },
        View { center: ViewCenter::Facing { azimuth: 0.0, tilt: 0.0 }, projection: ProjectionKind::Stereographic, fov_degrees: 120.0 },
        View { center: ViewCenter::Facing { azimuth: 3.9, tilt: 0.5 }, projection: ProjectionKind::Stereographic, fov_degrees: 20.0 },
        View { center: ViewCenter::Facing { azimuth: 1.5, tilt: -0.3 }, projection: ProjectionKind::Equidistant, fov_degrees: 300.0 },
        View { center: ViewCenter::Facing { azimuth: 5.0, tilt: 1.4 }, projection: ProjectionKind::Stereographic, fov_degrees: 1.0 },
    ];
    for (width, height) in [(401, 221), (13, 7), (1, 1), (0, 0)] {
        let mut frame = Frame::new(width, height, width.min(height), (10, 20));
        let mut terminal = Terminal::new(Vec::new(), ProtocolType::Sixel, false, Rect::new(0, 0, (width / 10).max(1) as u16, (height / 20).max(1) as u16), false, false);
        for (index, view) in views.iter().enumerate() {
            run_frame(&mut data, &mut frame, view, &settings());
            assert_eq!(frame.stars().len(), width * height);
            let presented = present(&frame, &mut terminal, &mut astroterm::time::StepTimes::default()).unwrap();                                  // 7
            let expect_sent = index == 0 && width * height > 0;                                       // the first frame of a size is always sent; tiny frames may then repeat
            assert!(!expect_sent || matches!(presented, Presented::Sent(_)), "{width}x{height} {view:?}: {presented:?}");
        }
    }
    let mut frame = Frame::new(401, 221, 221, (10, 20));
    let (_, drawn) = run_frame(&mut data, &mut frame, &views[0], &settings());
    let lit = frame.stars().iter().filter(|texel| texel.opacity > 0.0).count();
    assert!(lit > 800 && drawn.arcs > 50 && drawn.direction_labels.len() == 4, "lit={lit} {drawn:?}");    // the overhead frame has stars, figures and the compass
}

#[test]
fn dynamic_names_label_stars_in_a_zoomed_view() {
    let mut data = tables();
    let view = View { center: ViewCenter::Facing { azimuth: 225_f64.to_radians(), tilt: 30_f64.to_radians() }, projection: ProjectionKind::Stereographic, fov_degrees: 20.0 };
    let mut frame = Frame::new(401, 221, 221, (10, 20));
    let (_, named) = run_frame(&mut data, &mut frame, &view, &settings());
    let (_, unnamed) = run_frame(&mut data, &mut frame, &view, &RenderOptions { dynamic_names: false, ..settings() });
    assert!(named.star_labels.len() >= 3 && unnamed.star_labels.is_empty(), "{:?}", named.star_labels);
    assert!(named.star_labels.iter().all(|label| !label.text.is_empty()));
}

#[test]
fn polaris_is_lit_where_the_projection_places_it() {
    let mut data = tables();
    let view = View { center: ViewCenter::Zenith, projection: ProjectionKind::Stereographic, fov_degrees: 180.0 };
    let mut frame = Frame::new(401, 401, 401, (10, 20));
    let (observation, _) = run_frame(&mut data, &mut frame, &view, &settings());

    // Polaris is a constellation endpoint, so step 6 drew it from the constellation simulation
    let constellations = &data.constellation_stars;
    let row = (0..constellations.len()).find(|&row| star_label(constellations.labels(), row, &data.names, true) == "Polaris").expect("Polaris is in Ursa Minor");
    let direction = observation.apparent_star_direction(data.constellation_simulation.directions()[row]);
    let point = project_camera(prepare_camera(&view), direction).expect("Polaris is above Tokyo's horizon");
    let (r, c) = project_to_cell(frame.drawing_area_viewport(), point);
    assert!(r > 0 && r < 200 && c > 150 && c < 250, "Polaris at ({r}, {c}): north, above the centre");   // north is up in the zenith view
    let (row, column) = frame.drawing_area_pixel((r, c));
    let lit = (-2..=1).flat_map(|dr| (-2..=1).map(move |dc| (dr, dc))).any(|(dr, dc)| frame.star((row + dr) as usize, (column + dc) as usize).opacity > 0.0);
    assert!(lit, "no star light around Polaris at ({row}, {column})");
}
