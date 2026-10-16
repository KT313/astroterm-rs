use super::*;
use crate::formulas::{Observer, evaluate_earth_state};
use crate::model::{Data, ProjectionKind, View, ViewCenter};
use crate::observation::observe;
use crate::projection::prepare_camera;
use crate::projection::pixel_warp::{project_tiles_by_pixel, content_bounds_tests::build_dense_image};
use crate::projection::pixel_warp::row_stepping_tests::{compare_images, project_unshared_tiles};
use ProjectionKind::{Equidistant, Stereographic};

fn create_view(fov: f64, projection: ProjectionKind) -> View {
    View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, projection }
}

fn observe_view(view: &View, atmosphere: bool, offset_ms: i64, latitude: f64) -> Observation {
    let date = crate::formulas::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
    let time = crate::formulas::utc_jd_to_time_ms(crate::formulas::datetime_to_julian_date(&date)).unwrap() + offset_ms;
    let observer = Observer { latitude: latitude.to_radians(), longitude: 139.69_f64.to_radians() };
    observe(&observer, atmosphere, view, time, evaluate_earth_state(time), &mut StepTimes::default())
}

fn project_in_strips(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, strip_height: usize) -> Vec<Texel> {
    let mut times = StepTimes::default();
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size * size];
    if size < 2 { return stars; }
    times.measure("Projection.Camera preparation");
    let screen = Screen::new(prepare_camera(view), size);
    project_shared_tiles(&mut stars, &screen, regions, selected, observation, &mut times, strip_height);
    times.measure("Projection");
    stars
}

fn render_shared_fixture(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View, strip_height: usize) -> Vec<Texel> {
    // Exercise the real shared drawing kernel even when a small fixture would choose the workload fallback.
    let mut stars = vec![Texel::default(); size * size];
    let screen = Screen::new(prepare_camera(view), size);
    let mut times = StepTimes::default();
    let prepared = prepare_regions(&screen, regions, selected, observation, &mut times);
    let strips = index_regions_by_strip(&prepared, size, strip_height);
    draw_shared_strips(&mut stars, &screen, &prepared, &strips, observation, &mut times, strip_height);
    stars
}

#[test]
fn shared_preparation_calculates_each_disc_pixel_once_and_matches_the_direct_ray() {
    for (fov, projection, atmosphere) in [(180.0, Stereographic, true), (359.0, Stereographic, true),
        (60.0, Equidistant, false), (360.0, Equidistant, false), (360.0, Equidistant, true)]
    {
        let view = create_view(fov, projection);
        let observation = observe_view(&view, atmosphere, 0, 35.69);
        let screen = Screen::new(prepare_camera(&view), 1116);
        let corrections = ScreenCorrections::new(&observation);
        let mut shared = SharedScreenRows::new(screen.size, 32);
        let mut expected_pixels = 0;

        // Disjoint strip ranges must calculate each disc pixel exactly once, including the last short strip.
        for first_row in (0..screen.size).step_by(32) {
            let end_row = (first_row + 32).min(screen.size);
            shared.prepare(&screen, &corrections, first_row, end_row);
            for row in first_row..end_row {
                let Some((first, last)) = screen.disc_columns(row) else { continue; };
                expected_pixels += last - first + 1;
                for (offset, actual) in shared.span(row, first, last).iter().enumerate() {
                    let (numerator, spread, factor) = screen.ray((first + offset) as f64, row as f64);
                    let expected = corrections.calculate(numerator, spread, factor);
                    let tolerance = 1e-9 * spread.max(1.0);
                    assert!((actual.direction - expected.direction).length() < tolerance);
                    assert!((actual.length - expected.length).abs() < tolerance);
                    assert!((actual.factor - expected.factor).abs() < 1e-9);
                }
            }
        }
        assert_eq!(shared.prepared_pixels, expected_pixels);
        assert!(expected_pixels > screen.size * screen.size / 2);
    }
}

#[test]
#[ignore = "single-thread release benchmark; embedded BSC5 only"]
fn time_shared_screen_sparse_fallback() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut report = crate::progress::Report::quiet();
    let catalog = crate::catalog::load_embedded_catalog(&mut report).unwrap();
    let catalog = crate::sky::prepare_owned_catalog(catalog, &mut report).unwrap();
    let original = crate::sky::build_sky_tables(&catalog, &mut report).unwrap();
    for (fov, projection, atmosphere) in [(180.0, Stereographic, true), (351.6, Stereographic, true),
        (180.0, Equidistant, false), (360.0, Equidistant, false)]
    {
        let view = create_view(fov, projection);
        let observation = observe_view(&view, atmosphere, 0, 35.69);
        let mut data = original.clone();
        let selected = crate::selection::select_regions(&data.star_regions, &observation.patch, observation.time_ms, &mut StepTimes::default());
        crate::regions::refresh_regions(&mut data.star_regions, &data.constellation_stars,
            &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
            &mut data.star_opacities, &mut data.tile_bytes, &selected.regions, observation.time_ms,
            fov, 9.0, &mut StepTimes::default());
        let mut old_times = Vec::new();
        let mut shared_times = Vec::new();
        for round in 0..6 {
            for shared in if round % 2 == 0 { [false, true] } else { [true, false] } {
                let start = Instant::now();
                let stars = if shared {
                    project_tiles_by_pixel(1116, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default())
                } else {
                    project_unshared_tiles(1116, &data.star_regions, &selected.regions, &observation, &view)
                };
                black_box(&stars);
                if round > 0 {
                    if shared { shared_times.push(start.elapsed().as_secs_f64() * 1000.0); }
                    else { old_times.push(start.elapsed().as_secs_f64() * 1000.0); }
                }
            }
        }
        old_times.sort_by(f64::total_cmp);
        shared_times.sort_by(f64::total_cmp);
        let old = project_unshared_tiles(1116, &data.star_regions, &selected.regions, &observation, &view);
        let shared = project_tiles_by_pixel(1116, &data.star_regions, &selected.regions, &observation, &view, &mut StepTimes::default());
        eprintln!("Shared sparse {projection:?} fov={fov} atmosphere={atmosphere}: original {:.3} ms, shared {:.3} ms, error {}",
            old_times[2], shared_times[2], compare_images(&shared, &old));
    }
}

#[test]
fn strip_index_preserves_selection_order_and_avoids_unprofitable_sharing() {
    let view = create_view(180.0, Equidistant);
    let observation = observe_view(&view, false, 0, 35.69);
    let screen = Screen::new(prepare_camera(&view), 301);
    let mut data = Data::default();
    let selected: Vec<_> = (0..data.star_regions.len()).rev().step_by(7).collect();
    for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, true); }
    let prepared = prepare_regions(&screen, &data.star_regions, &selected, &observation, &mut StepTimes::default());
    let strips = index_regions_by_strip(&prepared, screen.size, 16);

    // Bucket entries are increasing ordinals in the supplied selection, not sorted physical region IDs.
    for (strip, indices) in strips.iter().enumerate() {
        assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
        for (index, region) in prepared.iter().enumerate() {
            let intersects = region.first_row < (strip + 1) * 16 && region.last_row >= strip * 16;
            assert_eq!(indices.contains(&index), intersects);
        }
    }
    assert!(has_repeated_screen_work(&prepared, &strips, &screen, 16));

    // A tiny isolated image must not cause us to prepare most of the drawing disc.
    let small = prepared.iter().position(|region| region.last_row - region.first_row < 10).unwrap();
    let one = &prepared[small..=small];
    let strips = index_regions_by_strip(one, screen.size, 32);
    assert!(!has_repeated_screen_work(one, &strips, &screen, 32));
}

#[test]
fn shared_strips_preserve_ordered_images_across_view_and_observer_changes() {
    let mut data = Data::default();
    for (fov, projection, atmosphere, offset_ms, latitude) in [
        (1.0, Stereographic, true, 0, 35.69),
        (60.0, Equidistant, false, 0, 35.69),
        (180.0, Stereographic, true, 0, 35.69),
        (359.0, Stereographic, true, -86_400_000, 78.0),
        (360.0, Equidistant, true, 86_400_000, -45.0),
        (360.0, Equidistant, false, 0, 35.69),
        (180.0, Stereographic, false, 0, 35.69),
    ] {
        let mut view = create_view(fov, projection);
        if offset_ms != 0 {
            view.center = ViewCenter::Facing { azimuth: 1.7, tilt: if offset_ms > 0 { 0.37 } else { -0.22 } };
        }
        let observation = observe_view(&view, atmosphere, offset_ms, latitude);

        // A few overlapping front and rear regions cover ordering, seams and the antipodal rim.
        let centre = observation.patch.center;
        let region = crate::model::hash_direction(crate::constants::GRID_DEPTH, centre);
        let tile = TileGeometry::new(region);
        let mut selected = vec![0, data.star_regions.len() - 1];
        for direction in [centre, centre + tile.east() * 0.06, centre + tile.north() * 0.06,
            -centre, -centre + tile.east() * 0.06, -centre + tile.north() * 0.06]
        {
            selected.push(crate::model::hash_direction(crate::constants::GRID_DEPTH, direction));
        }
        selected.sort_unstable();
        selected.dedup();
        selected.reverse();
        for (ordinal, &region) in selected.iter().enumerate() {
            data.star_regions[region].merged_tile = build_dense_image(region, true);
            for texel in &mut data.star_regions[region].merged_tile.tile.texels {
                texel.rgb[ordinal % 3] *= 0.2;
            }
            data.star_regions[region].merged_tile.filter_summary = None;
        }
        for size in [0, 1, 101, 301] {
            let reference = project_unshared_tiles(size, &data.star_regions, &selected, &observation, &view);
            let production = project_tiles_by_pixel(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            let error = compare_images(&production, &reference);
            if !atmosphere && projection == Stereographic { assert_eq!(error, 0.0); }
            if size < 2 { continue; }

            // The narrow view also exercises shared dispatch through the normal production entry point.
            if size == 301 && fov == 1.0 {
                let screen = Screen::new(prepare_camera(&view), size);
                let prepared = prepare_regions(&screen, &data.star_regions, &selected, &observation, &mut StepTimes::default());
                let strips = index_regions_by_strip(&prepared, size, 32);
                assert!(has_repeated_screen_work(&prepared, &strips, &screen, 32));
            }

            // Full-height is the region-major shared reference; short strips must not introduce seams.
            let full = render_shared_fixture(size, &data.star_regions, &selected, &observation, &view, size);
            compare_images(&full, &reference);
            for height in [1, 16, 32] {
                let actual = render_shared_fixture(size, &data.star_regions, &selected, &observation, &view, height);
                compare_images(&actual, &full);
            }
        }
    }
}

#[test]
fn empty_selection_and_empty_images_leave_no_shared_output() {
    let view = create_view(360.0, Equidistant);
    let observation = observe_view(&view, true, 0, 35.69);
    let data = Data::default();
    for selected in [vec![], vec![0, 1, 2]] {
        let stars = project_tiles_by_pixel(31, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        assert!(stars.iter().all(|pixel| pixel.is_empty()));
    }
}

#[test]
#[ignore = "single-thread release benchmark; synthetic dense tiles only"]
fn time_shared_screen_strip_sizes() {
    benchmark_shared_screen(&[0, 16, 32, 64, 1116], &[false, true], &[
        (180.0, Stereographic, false), (180.0, Stereographic, true), (351.6, Stereographic, true),
        (180.0, Equidistant, false), (360.0, Equidistant, false), (360.0, Equidistant, true),
    ]);
}

#[test]
#[ignore = "single-thread release benchmark of the selected strip size; synthetic tiles only"]
fn time_shared_screen_selected_path() {
    benchmark_shared_screen(&[0, crate::constants::SHARED_SCREEN_STRIP_ROWS], &[false], &[
        (180.0, Stereographic, true), (360.0, Equidistant, false), (360.0, Equidistant, true),
    ]);
}

fn benchmark_shared_screen(heights: &[usize], full_cases: &[bool], views: &[(f64, ProjectionKind, bool)]) {
    use std::hint::black_box;
    use std::time::Instant;

    let mut data = Data::default();
    eprintln!("Shared ray bytes={}, prepared region bytes={}", std::mem::size_of::<CorrectedRay>(), std::mem::size_of::<PreparedRegion<'_>>());
    let selected: Vec<_> = (0..data.star_regions.len()).collect();
    for &full in full_cases {
        for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, full); }
        for &(fov, projection, atmosphere) in views {
            let view = create_view(fov, projection);
            let observation = observe_view(&view, atmosphere, 0, 35.69);
            let mut timings = vec![Vec::new(); heights.len()];
            for round in 0..6 {
                for offset in 0..heights.len() {
                    let index = if round % 2 == 0 { offset } else { heights.len() - 1 - offset };
                    let start = Instant::now();
                    let stars = if heights[index] == 0 {
                        project_unshared_tiles(1116, &data.star_regions, &selected, &observation, &view)
                    } else {
                        project_in_strips(1116, &data.star_regions, &selected, &observation, &view, heights[index])
                    };
                    black_box(&stars);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if round > 0 { timings[index].push(elapsed); }
                }
            }

            // Validate full compositions outside the timed loops.
            let reference = project_unshared_tiles(1116, &data.star_regions, &selected, &observation, &view);
            for (index, &height) in heights.iter().enumerate() {
                timings[index].sort_by(f64::total_cmp);
                let error = if height == 0 { 0.0 } else {
                    let actual = project_in_strips(1116, &data.star_regions, &selected, &observation, &view, height);
                    compare_images(&actual, &reference)
                };
                eprintln!("Shared screen full={full} {projection:?} fov={fov} atmosphere={atmosphere} strip={height}: {:.3} ms (min {:.3}, max {:.3}), RGBA error {error}",
                    timings[index][2], timings[index][0], timings[index][4]);
            }
        }
    }
}
