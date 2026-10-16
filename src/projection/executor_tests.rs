use super::*;
use crate::constants::{GRID_DEPTH, TILE_EDGE_TEXELS};
use crate::formulas::{Observer, evaluate_earth_state};
use crate::model::{Data, ProjectionKind, ViewCenter, hash_direction};
use crate::observation::observe;
use crate::regions::TileGeometry;
use super::super::pixel_warp::content_bounds_tests::{build_dense_image, merge_image};
use super::super::pixel_warp::row_stepping_tests::compare_images;
use ProjectionKind::{Equidistant, Stereographic};

fn create_view(fov: f64, projection: ProjectionKind) -> View {
    View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, projection }
}

fn observe_view(view: &View, atmosphere: bool) -> Observation {
    let time = crate::time::parse_simulation_time_ms("2025-03-01T11:00:00").unwrap();
    let observer = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
    observe(&observer, atmosphere, view, time, evaluate_earth_state(time), &mut StepTimes::default())
}

fn fill_overlapping_regions(data: &mut Data, observation: &Observation) -> Vec<usize> {
    // Place coloured tiles around both the view centre and its antipode, with fully lit padding.
    let centre = observation.patch.center;
    let region = hash_direction(GRID_DEPTH, centre);
    let tile = TileGeometry::new(region);
    let mut selected = Vec::new();
    for direction in [centre, centre + tile.east() * 0.02, centre - tile.east() * 0.02,
        centre + tile.north() * 0.02, centre - tile.north() * 0.02, -centre,
        -centre + tile.east() * 0.02, -centre - tile.north() * 0.02]
    {
        selected.push(hash_direction(GRID_DEPTH, direction));
    }
    selected.sort_unstable();
    selected.dedup();
    selected.reverse();
    for (ordinal, &region) in selected.iter().enumerate() {
        let opacity = if ordinal == 0 { 1.0 } else { 0.3 + ordinal as f32 * 0.05 };
        let mut rgb = [opacity * 0.1; 3];
        rgb[ordinal % 3] = opacity;
        data.star_regions[region].merged_tile = merge_image(vec![Texel { rgb, opacity }; TILE_EDGE_TEXELS.pow(2)]);
    }
    selected
}

#[test]
fn direct_mode_has_no_pool_and_explicit_counts_are_not_capped_to_cpus() {
    let direct = ProjectionExecutor::new(0).unwrap();
    assert!(direct.pool.is_none());
    assert!(direct.buffers.is_empty());
    for workers in [1, 2, 4] {
        let executor = ProjectionExecutor::new(workers).unwrap();
        let pool = executor.pool.as_ref().unwrap();
        assert_eq!(pool.current_num_threads(), workers);
        let caller = std::thread::current().id();
        pool.in_place_scope(|scope| {
            assert_eq!(std::thread::current().id(), caller);
            scope.spawn(move |_| {
                assert_ne!(std::thread::current().id(), caller);
                assert!(std::thread::current().name().unwrap().starts_with("projection-"));
            });
        });
    }
    assert!(ProjectionExecutor::new(rayon::max_num_threads() + 1).is_err());
}

#[test]
fn chunks_preserve_order_are_balanced_and_never_empty() {
    for length in 0..32 {
        let regions: Vec<_> = (0..length).rev().collect();
        for workers in [1, 2, 3, 8, 40] {
            let chunks = split_region_chunks(&regions, workers);
            assert_eq!(chunks.len(), workers.min(length));
            assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
            let joined: Vec<_> = chunks.iter().flat_map(|chunk| chunk.iter().copied()).collect();
            assert_eq!(joined, regions);
            if let (Some(first), Some(last)) = (chunks.first(), chunks.last()) {
                assert!(first.len() - last.len() <= 1);
            }
        }
    }
}

#[test]
fn pool_matches_direct_for_overlapping_tiles_and_reused_images() {
    let mut data = Data::default();
    let mut executors: Vec<_> = [0, 1, 2, 4, 12].map(|count| ProjectionExecutor::new(count).unwrap()).into();
    for (fov, projection, atmosphere) in [(1.0, Stereographic, false), (180.0, Stereographic, false),
        (359.0, Stereographic, false), (180.0, Stereographic, true), (359.0, Stereographic, true),
        (60.0, Equidistant, false), (360.0, Equidistant, false), (360.0, Equidistant, true)]
    {
        let view = create_view(fov, projection);
        let observation = observe_view(&view, atmosphere);
        let mut selected = fill_overlapping_regions(&mut data, &observation);
        for size in [0, 1, 67, 129, 35] {
            let expected = ProjectionExecutor::default().project(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            for executor in &mut executors {
                let actual = executor.project(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
                let error = compare_images(&actual, &expected);
                if executor.pool.as_ref().is_none_or(|pool| pool.current_num_threads() == 1) {
                    assert_eq!(error, 0.0, "direct and one-worker paths retain the same arithmetic order");
                }
            }
        }

        // Reversing region order must reverse composition order, regardless of task completion order.
        selected.reverse();
        let expected = ProjectionExecutor::default().project(129, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        for executor in &mut executors {
            let actual = executor.project(129, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            compare_images(&actual, &expected);
        }

        // Replacing tiles and rendering empty selections must not retain a previous frame's light.
        for &region in &selected { data.star_regions[region].merged_tile = Default::default(); }
        for executor in &mut executors {
            for selection in [&selected[..], &[][..]] {
                let actual = executor.project(67, &data.star_regions, selection, &observation, &view, &mut StepTimes::default());
                assert!(actual.iter().all(|pixel| *pixel == Texel::default()));
            }
        }
    }
}

#[test]
fn one_worker_writes_the_final_image_without_temporary_images_or_composition() {
    let view = create_view(180.0, Stereographic);
    let observation = observe_view(&view, false);
    let mut data = Data::default();
    let selected = fill_overlapping_regions(&mut data, &observation);
    let mut executor = ProjectionExecutor::new(1).unwrap();

    // A real worker performs the same region loop; its image is returned directly, even after resizing.
    for size in [67, 129, 35] {
        let mut times = StepTimes::with_trace(true);
        let actual = executor.project(size, &data.star_regions, &selected, &observation, &view, &mut times);
        times.measure("END");
        let expected = ProjectionExecutor::default().project(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        assert_eq!(actual, expected);
        assert!(executor.buffers.is_empty());
        assert!(times.steps().iter().any(|step| step.name == "Worker rasterization"));
        assert!(!times.steps().iter().any(|step| step.name == "Worker composition"));
    }
}

#[test]
fn ordered_chunk_composition_matches_serial_including_saturated_pixels() {
    // RGB remains premultiplied, including opaque white and very faint contributions.
    let sources: Vec<_> = (0..1000).map(|index| {
        let alpha = if index % 17 == 0 { 1.0 } else { (index % 31) as f32 / 31.0 };
        Texel { rgb: [alpha, alpha * (index % 13) as f32 / 13.0, alpha * 1e-8], opacity: alpha }
    }).collect();
    let mut reference = Texel::default();
    for source in &sources { blend_star(&mut reference, source.rgb, source.opacity); }
    for workers in [1, 2, 3, 8, 1000] {
        let mut buffers = Vec::new();
        for chunk in split_region_chunks(&sources, workers) {
            let mut pixel = Texel::default();
            for source in chunk { blend_star(&mut pixel, source.rgb, source.opacity); }
            buffers.push(vec![pixel]);
        }
        let mut actual = [buffers[0][0]];
        combine_chunk_buffers(&mut actual, &buffers[1..]);
        compare_images(&actual, &[reference]);
    }
}

#[test]
fn chunk_layers_must_be_composed_in_selection_order() {
    let first = Texel { rgb: [0.5, 0.0, 0.0], opacity: 0.5 };
    let second = Texel { rgb: [0.0, 0.0, 0.75], opacity: 0.75 };
    let mut forward = [first];
    let mut reverse = [second];
    combine_chunk_buffers(&mut forward, &[vec![second]]);
    combine_chunk_buffers(&mut reverse, &[vec![first]]);
    assert_eq!(forward[0].rgb, [0.125, 0.0, 0.75]);
    assert_eq!(reverse[0].rgb, [0.5, 0.0, 0.375]);
    assert_eq!(forward[0].opacity, 0.875);
}

#[test]
fn worker_panics_propagate_and_scoped_jobs_finish_before_return() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let executor = ProjectionExecutor::new(2).unwrap();
    let finished = AtomicBool::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        executor.pool.as_ref().unwrap().in_place_scope(|scope| {
            scope.spawn(|_| panic!("injected worker panic"));
            scope.spawn(|_| finished.store(true, Ordering::SeqCst));
        });
    }));
    assert!(result.is_err());
    assert!(finished.load(Ordering::SeqCst));
}

#[test]
fn worker_timings_report_wall_stages_without_changing_output() {
    let view = create_view(1.0, Stereographic);
    let observation = observe_view(&view, true);
    let mut data = Data::default();
    let selected = fill_overlapping_regions(&mut data, &observation);
    let mut executor = ProjectionExecutor::new(2).unwrap();
    let mut times = StepTimes::with_trace(true);
    times.begin_frame();
    let actual = super::super::project_stars_to_screen(67, &data.star_regions, &selected, &observation, &view, &mut executor, &mut times);
    times.measure("END");
    for name in ["Worker preparation", "Worker rasterization", "Worker composition"] {
        assert!(times.steps().iter().any(|step| step.name == name && step.depth == 1), "missing {name}");
    }
    let untraced = executor.project(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    assert_eq!(actual, untraced);
}

#[test]
#[ignore = "release projection pool benchmark; synthetic dense tiles, no downloads"]
fn time_projection_workers() {
    use std::hint::black_box;
    use std::time::Instant;

    // Keep source tiles and thread pools alive across frames, with no dataset or terminal work in timing.
    let mut data = Data::default();
    let selected: Vec<_> = (0..data.star_regions.len()).collect();
    for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, true); }
    let views = [create_view(180.0, Stereographic), create_view(359.0, Stereographic)];
    let observations = views.each_ref().map(|view| observe_view(view, false));
    let jobs = [0, 1, 4];
    let mut executors = jobs.map(|count| ProjectionExecutor::new(count).unwrap());
    let mut references = Vec::new();
    for (view, observation) in views.iter().zip(&observations) {
        references.push(ProjectionExecutor::default().project(1116, &data.star_regions, &selected, observation, view, &mut StepTimes::default()));
    }

    // Rotate all six combinations through every position: two warmups, then twelve measured rounds.
    let mut samples = vec![Vec::new(); 6];
    let mut maximum_error = [0.0_f32; 6];
    for round in 0..14 {
        for offset in 0..6 {
            let case = (round + offset) % 6;
            let view_index = case / 3;
            let worker_index = case % 3;
            let start = Instant::now();
            let actual = executors[worker_index].project(1116, &data.star_regions, &selected,
                &observations[view_index], &views[view_index], &mut StepTimes::default());
            black_box(&actual);
            let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
            if round >= 2 { samples[case].push(milliseconds); }

            // Validate the entire image outside timing, including ordered multi-worker composition.
            maximum_error[case] = maximum_error[case].max(compare_images(&actual, &references[view_index]));
            if jobs[worker_index] <= 1 { assert_eq!(actual, references[view_index]); }
            eprintln!("Independent regions round={round} fov={} jobs={} ms={milliseconds:.3}",
                views[view_index].fov_degrees, jobs[worker_index]);
        }
    }

    // Report medians and ranges so a noisy run cannot be mistaken for a consistent improvement.
    for (case, values) in samples.iter_mut().enumerate() {
        values.sort_by(f64::total_cmp);
        let median = (values[5] + values[6]) * 0.5;
        eprintln!("Independent regions summary fov={} jobs={} median={median:.3} range={:.3}..{:.3} max_rgba_error={}",
            views[case / 3].fov_degrees, jobs[case % 3], values[0], values[11], maximum_error[case]);
    }
}
