use super::*;
use super::super::screen_blend::{blend_screen_images, blend_screen_pixel};
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

#[test]
fn bilinear_switch_preserves_images_in_both_views_and_worker_modes() {
    for kind in [Stereographic, Equidistant] {
        for atmosphere in [false, true] {
            let view = create_view(1.0, kind);
            let observation = observe_view(&view, atmosphere);
            let mut data = Data::default();
            let selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));

            // Keep a separate baseline executor so its borrowed image survives every candidate run.
            let mut baseline = ProjectionExecutor::default();
            baseline.options.bilinear_fast_path = false;
            let expected = baseline.project(101, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            for workers in [0, 1, 4] {
                let mut executor = ProjectionExecutor::new(workers).unwrap();
                for enabled in [true, false, true] {
                    executor.options.bilinear_fast_path = enabled;
                    let actual = executor.project(101, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
                    compare_images(actual, expected);
                    assert!(actual.iter().zip(expected).all(|(a, b)| a.is_empty() == b.is_empty()));
                }
            }
        }
    }
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
    assert_eq!(direct.buffers.len(), 1);
    assert!(direct.buffers[0].pixels.is_empty());
    for workers in [1, 2, 4] {
        let executor = ProjectionExecutor::new(workers).unwrap();
        let pool = executor.pool.as_ref().unwrap();
        assert_eq!(pool.current_num_threads(), workers);
        assert_eq!(executor.buffers.len(), workers);
        assert!(executor.buffers.iter().all(|image| image.pixels.is_empty()));
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
            let mut chunks = Vec::new();
            split_region_chunks(&mut chunks, regions.len(), workers);
            assert_eq!(chunks.len(), workers.min(length));
            assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
            let joined: Vec<_> = chunks.iter().flat_map(|chunk| regions[chunk.clone()].iter().copied()).collect();
            assert_eq!(joined, regions);
            if let (Some(first), Some(last)) = (chunks.first(), chunks.last()) {
                assert!(first.len() - last.len() <= 1);
            }
        }
    }
}

#[test]
fn weighted_chunks_keep_every_region_in_order_and_spread_expensive_suffixes() {
    let regions: Vec<_> = (0..16).collect();
    let mut work = vec![1.0; 16];
    work[12..].fill(100.0);
    let mut chunks = Vec::new();
    split_regions_by_work(&mut chunks, &work, 4);
    let costs: Vec<f64> = chunks.iter().map(|chunk| work[chunk.clone()].iter().sum()).collect();
    assert_eq!(costs, [112.0, 100.0, 100.0, 100.0]);
    let mut equal_chunks = Vec::new();
    split_region_chunks(&mut equal_chunks, regions.len(), 4);
    let old_costs: Vec<f64> = equal_chunks.iter().map(|chunk| work[chunk.clone()].iter().sum()).collect();
    assert_eq!(old_costs, [4.0, 4.0, 4.0, 400.0]);

    // Include empty inputs, zero/uniform costs, and a dominant region at either end of the list.
    for length in 0..32 {
        let regions: Vec<_> = (0..length).collect();
        for pattern in 0..5 {
            let work: Vec<_> = (0..length).map(|index| match pattern {
                0 => 0.0,
                1 => 1.0,
                2 if index == 0 => 1e12,
                3 if index + 1 == length => 1e12,
                _ => (index * 137 % 19 + 1) as f64,
            }).collect();
            for workers in [1, 2, 3, 8, 40] {
                let mut chunks = Vec::new();
                split_regions_by_work(&mut chunks, &work, workers);
                assert_eq!(chunks.len(), workers.min(length));
                assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
                let joined: Vec<_> = chunks.iter().flat_map(|chunk| regions[chunk.clone()].iter().copied()).collect();
                assert_eq!(joined, regions);
            }
        }
    }
}

#[test]
fn rough_estimates_are_bounded_and_detect_stereographic_enlargement() {
    let view = create_view(359.0, Stereographic);
    let mut observation = observe_view(&view, false);
    let mut data = Data::default();
    data.star_regions[0].merged_tile = build_dense_image(0, true);
    let tiles = select_region_tiles(&data.star_regions, &Selected::of_regions(vec![0, 1]));
    let centre = crate::model::region_caps()[0].center;
    let scale = prepare_camera(&view).scale;

    observation.patch.center = centre;
    let mut forward = Vec::new();
    estimate_stereographic_work(&mut forward, &tiles, &observation.patch.center, scale, 1116, view.fov_degrees);
    observation.patch.center = -centre;
    let mut rear = Vec::new();
    estimate_stereographic_work(&mut rear, &tiles, &observation.patch.center, scale, 1116, view.fov_degrees);
    assert_eq!(forward[1], 1.0); // An empty image keeps only the baseline visit cost.
    assert_eq!(rear[1], 1.0);
    assert!(rear[0] > forward[0] * 1000.0);
    for cost in forward.into_iter().chain(rear) {
        assert!(cost.is_finite() && (1.0..=(1116.0 * 1116.0)).contains(&cost));
    }
}

#[test]
fn only_multi_worker_stereographic_views_estimate_work() {
    let data = Data::default();
    for projection in [Stereographic, Equidistant] {
        let view = create_view(180.0, projection);
        let observation = observe_view(&view, false);
        for workers in [0, 1, 4] {
            for selected in [Selected::of_regions(vec![0]), Selected::of_regions(vec![0, 1])] {
                let mut executor = ProjectionExecutor::new(workers).unwrap();
                let mut times = StepTimes::with_trace(true);
                executor.project(33, &data.star_regions, &selected, &observation, &view, &mut times);
                times.measure("END");
                let estimated = times.steps().iter().any(|step| step.name == "Work estimation");
                assert_eq!(estimated, workers > 1 && projection == Stereographic && selected.regions.len() > 1);
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
        let mut selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));
        for size in [0, 1, 67, 129, 35] {
            let expected = project_test_image(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            for executor in &mut executors {
                let actual = executor.project(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
                let error = compare_images(actual, &expected);
                if executor.pool.as_ref().is_none_or(|pool| pool.current_num_threads() == 1) {
                    assert_eq!(error, 0.0, "direct and one-worker paths retain the same arithmetic order");
                }
            }
        }

        // Screen blending also preserves the image when the selected regions themselves are reversed.
        let forward = project_test_image(129, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        selected.regions.reverse();
        let expected = project_test_image(129, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        compare_images(&expected, &forward);
        for executor in &mut executors {
            let actual = executor.project(129, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            compare_images(actual, &expected);
        }

        // Replacing tiles and rendering empty selections must not retain a previous frame's light.
        for &region in &selected.regions { data.star_regions[region].merged_tile = Default::default(); }
        for executor in &mut executors {
            for selection in [&selected, &Selected::default()] {
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
    let selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));
    let mut executor = ProjectionExecutor::new(1).unwrap();

    // A real worker performs the same region loop; its image is returned directly, even after resizing.
    for size in [67, 129, 35] {
        let mut times = StepTimes::with_trace(true);
        let actual = executor.project(size, &data.star_regions, &selected, &observation, &view, &mut times);
        times.measure("END");
        let expected = project_test_image(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        assert_eq!(actual, expected);
        let output_address = actual.as_ptr();
        assert_eq!(executor.buffers.len(), 1);
        assert_eq!(executor.buffers[0].pixels.as_ptr(), output_address);
        assert!(times.steps().iter().any(|step| step.name == "Worker wait"));
        assert!(!times.steps().iter().any(|step| step.name == "Worker composition"));
    }
}

#[test]
fn arbitrary_chunk_composition_matches_serial_including_saturated_pixels() {
    // RGB remains premultiplied, including opaque white and very faint contributions.
    let sources: Vec<_> = (0..1000).map(|index| {
        let alpha = if index % 17 == 0 { 1.0 } else { (index % 31) as f32 / 31.0 };
        Texel { rgb: [alpha, alpha * (index % 13) as f32 / 13.0, alpha * 1e-8], opacity: alpha }
    }).collect();
    let mut reference = Texel::default();
    for source in &sources { blend_screen_pixel(&mut reference, *source); }
    for workers in [1, 2, 3, 8, 1000] {
        let mut buffers = Vec::new();
        let mut chunks = Vec::new();
        split_region_chunks(&mut chunks, sources.len(), workers);
        for range in chunks {
            let chunk = &sources[range];
            let mut pixel = Texel::default();
            for source in chunk { blend_screen_pixel(&mut pixel, *source); }
            buffers.push(vec![pixel]);
        }
        let mut actual = [buffers[0][0]];
        for buffer in buffers[1..].iter().rev() { blend_screen_images(&mut actual, buffer); }
        compare_images(&actual, &[reference]);
    }
}

#[test]
fn chunk_layers_can_be_composed_in_either_order() {
    let first = Texel { rgb: [0.5, 0.0, 0.0], opacity: 0.5 };
    let second = Texel { rgb: [0.0, 0.0, 0.75], opacity: 0.75 };
    let mut forward = [first];
    let mut reverse = [second];
    blend_screen_images(&mut forward, &[second]);
    blend_screen_images(&mut reverse, &[first]);
    assert_eq!(forward[0].rgb, [0.5, 0.0, 0.75]);
    assert_eq!(reverse, forward);
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
fn drawing_failure_returns_every_buffer_before_propagating_and_pool_can_be_reused() {
    let view = create_view(1.0, Stereographic);
    let observation = observe_view(&view, false);
    let mut data = Data::default();
    let selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));
    let centre = hash_direction(GRID_DEPTH, observation.patch.center);
    let valid_image = data.star_regions[centre].merged_tile.clone();
    data.star_regions[centre].merged_tile.tile.texels.truncate(1); // Real sampling must reject this malformed tile.
    let mut executor = ProjectionExecutor::new(4).unwrap();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        executor.project(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    }));
    assert!(failure.is_err());
    assert!(executor.buffers.iter().take(selected.regions.len()).all(|buffer| buffer.pixels.len() == 67 * 67));

    data.star_regions[centre].merged_tile = valid_image;
    let expected = project_test_image(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    let actual = executor.project(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    compare_images(actual, &expected);
    let addresses: Vec<_> = executor.buffers.iter().map(|buffer| buffer.pixels.as_ptr()).collect();
    executor.project(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    assert_eq!(addresses, executor.buffers.iter().map(|buffer| buffer.pixels.as_ptr()).collect::<Vec<_>>());
}

#[test]
fn scheduling_and_worker_timer_storage_survive_empty_frames() {
    let view = create_view(180.0, Stereographic);
    let observation = observe_view(&view, false);
    let mut data = Data::default();
    let selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));
    let mut executor = ProjectionExecutor::new(4).unwrap();
    let mut times = StepTimes::default();
    executor.project(67, &data.star_regions, &selected, &observation, &view, &mut times);
    let weights = (executor.work_weights.as_ptr(), executor.work_weights.capacity());
    let chunks = (executor.region_chunks.as_ptr(), executor.region_chunks.capacity());
    let timer_addresses: Vec<_> = executor.worker_times.iter().map(|timer| timer.steps().as_ptr()).collect();
    assert!(executor.worker_times.iter().all(|timer| timer.steps().len() == 1));

    // An empty frame uses just one task; returning to the full selection reuses every task's scratch.
    executor.project(67, &data.star_regions, &Selected::default(), &observation, &view, &mut times);
    executor.project(67, &data.star_regions, &selected, &observation, &view, &mut times);
    assert_eq!(weights, (executor.work_weights.as_ptr(), executor.work_weights.capacity()));
    assert_eq!(chunks, (executor.region_chunks.as_ptr(), executor.region_chunks.capacity()));
    assert_eq!(timer_addresses, executor.worker_times.iter().map(|timer| timer.steps().as_ptr()).collect::<Vec<_>>());
    assert!(executor.worker_times.iter().all(|timer| timer.steps().len() == 1));
}

#[test]
fn worker_timings_report_wall_stages_without_changing_output() {
    let view = create_view(1.0, Stereographic);
    let observation = observe_view(&view, true);
    let mut data = Data::default();
    let selected = Selected::of_regions(fill_overlapping_regions(&mut data, &observation));
    let mut executor = ProjectionExecutor::new(2).unwrap();
    let mut times = StepTimes::with_trace(true);
    times.begin_frame();
    let actual = super::super::project_stars_to_screen(67, &data.star_regions, &selected, &observation, &view, &mut executor, &mut times).to_vec();
    times.measure("END");
    for name in ["Worker dispatch", "Worker wait", "Worker composition"] {
        assert!(times.steps().iter().any(|step| step.name == name && step.depth == 1), "missing {name}");
    }
    let untraced = executor.project(67, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
    compare_images(&actual, untraced);
}

#[test]
#[ignore = "release projection pool benchmark; synthetic dense tiles, no downloads"]
fn time_projection_workers() {
    use std::hint::black_box;
    use std::time::Instant;

    // Keep source tiles and thread pools alive across frames, with no dataset or terminal work in timing.
    let mut data = Data::default();
    let selected = Selected::of_regions((0..data.star_regions.len()).collect());
    for &region in &selected.regions { data.star_regions[region].merged_tile = build_dense_image(region, true); }
    let views = [create_view(180.0, Stereographic), create_view(359.0, Stereographic)];
    let observations = views.each_ref().map(|view| observe_view(view, false));
    let jobs = [0, 1, 4, 24];
    let mut executors = jobs.map(|count| ProjectionExecutor::new(count).unwrap());
    let mut references = Vec::new();
    for (view, observation) in views.iter().zip(&observations) {
        references.push(project_test_image(1116, &data.star_regions, &selected, observation, view, &mut StepTimes::default()));
    }

    // Rotate every combination through all positions twice after two warmup rounds.
    let cases = views.len() * jobs.len();
    let mut samples = vec![Vec::new(); cases];
    let mut maximum_error = vec![0.0_f32; cases];
    for round in 0..(2 + cases * 2) {
        for offset in 0..cases {
            let case = (round + offset) % cases;
            let view_index = case / jobs.len();
            let worker_index = case % jobs.len();
            let start = Instant::now();
            let actual = executors[worker_index].project(1116, &data.star_regions, &selected,
                &observations[view_index], &views[view_index], &mut StepTimes::default());
            black_box(&actual);
            let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
            if round >= 2 { samples[case].push(milliseconds); }

            // Validate the entire image outside timing, including different completion orders.
            maximum_error[case] = maximum_error[case].max(compare_images(actual, &references[view_index]));
            if jobs[worker_index] <= 1 { assert_eq!(actual, references[view_index]); }
            eprintln!("Independent regions round={round} fov={} jobs={} ms={milliseconds:.3}",
                views[view_index].fov_degrees, jobs[worker_index]);
        }
    }

    // Report medians and ranges so a noisy run cannot be mistaken for a consistent improvement.
    for (case, values) in samples.iter_mut().enumerate() {
        values.sort_by(f64::total_cmp);
        let middle = values.len() / 2;
        let median = (values[middle - 1] + values[middle]) * 0.5;
        eprintln!("Independent regions summary fov={} jobs={} median={median:.3} range={:.3}..{:.3} max_rgba_error={}",
            views[case / jobs.len()].fov_degrees, jobs[case % jobs.len()], values[0], values[values.len() - 1], maximum_error[case]);
    }
}

/// The previous equal-count scheduler, retained only as a benchmark reference using the live draw kernel.
fn project_equal_count_reference<'a>(executor: &'a mut ProjectionExecutor, size: usize, regions: &[Region], selected: &Selected, observation: &Observation, view: &View) -> &'a [Texel] {
    let mut times = StepTimes::default();
    times.measure("Projection.Camera preparation");
    let screen = Screen::new(prepare_camera(view), size);
    let tiles = select_region_tiles(regions, selected);
    let pool = executor.pool.as_ref().unwrap();
    let mut chunks = Vec::new();
    split_region_chunks(&mut chunks, tiles.len(), pool.current_num_threads());
    times.measure("Projection.Worker dispatch");
    let result_slot = draw_worker_images(pool, &tiles, &chunks, &mut executor.buffers, &mut executor.worker_times, size, &screen, observation, &executor.options, &mut times);
    &executor.buffers[result_slot].pixels
}

#[test]
#[ignore = "release comparison of equal-count and rough-area scheduling; synthetic tiles only"]
fn time_stereographic_distribution() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut data = Data::default();
    let selected = Selected::of_regions((0..data.star_regions.len()).collect());
    for &region in &selected.regions { data.star_regions[region].merged_tile = build_dense_image(region, true); }
    let mut executor = ProjectionExecutor::new(4).unwrap();
    for fov in [180.0, 359.0] {
        let view = create_view(fov, Stereographic);
        let observation = observe_view(&view, false);
        let reference = project_test_image(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
        let mut measurements = [Vec::new(), Vec::new()];
        let mut maximum_error = [0.0_f32; 2];

        // Alternate schedulers with the same persistent pool and output capacity; compare pixels untimed.
        for round in 0..14 {
            for mode in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                let start = Instant::now();
                let actual = if mode == 0 {
                    project_equal_count_reference(&mut executor, 1116, &data.star_regions, &selected, &observation, &view)
                } else {
                    executor.project(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default())
                };
                black_box(&actual);
                let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
                if round >= 2 { measurements[mode].push(milliseconds); }
                maximum_error[mode] = maximum_error[mode].max(compare_images(actual, &reference));
            }
        }
        for (mode, samples) in measurements.iter_mut().enumerate() {
            samples.sort_by(f64::total_cmp);
            eprintln!("Distribution fov={fov} weighted={} median={:.3} range={:.3}..{:.3} max_rgba_error={}",
                mode != 0, (samples[5] + samples[6]) * 0.5, samples[0], samples[11], maximum_error[mode]);
        }

        // Measure the entire estimate and split, reusing the same owned scratch as the executor.
        let tiles = select_region_tiles(&data.star_regions, &selected);
        let camera = prepare_camera(&view);
        let mut work = Vec::new();
        let mut chunks = Vec::new();
        for round in 0..4 {
            let start = Instant::now();
            for _ in 0..1000 {
                estimate_stereographic_work(&mut work, black_box(&tiles), black_box(&observation.patch.center), camera.scale, 1116, fov);
                split_regions_by_work(&mut chunks, black_box(&work), 4);
                black_box(&chunks);
            }
            eprintln!("Estimate and split fov={fov} round={round}: {:.3} us for {} regions",
                start.elapsed().as_secs_f64() * 1e6 / 1000.0, tiles.len());
        }
        estimate_stereographic_work(&mut work, &tiles, &observation.patch.center, camera.scale, 1116, fov);
        split_regions_by_work(&mut chunks, &work, 4);
        eprintln!("Weighted region counts fov={fov}: {:?}", chunks.iter().map(|chunk| chunk.len()).collect::<Vec<_>>());
    }
}
