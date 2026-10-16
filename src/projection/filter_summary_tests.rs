//! Paired measurements of derived filtering data through the complete current projection pipeline.
use crate::formulas::{Observer, evaluate_earth_state};
use crate::model::{Data, ProjectionKind, View, ViewCenter};
use crate::observation::observe;
use crate::time::StepTimes;
use super::pixel_warp::content_bounds_tests::build_dense_image;
use super::executor::project_test_image;
use super::pixel_warp::row_stepping_tests::compare_images;
use ProjectionKind::{Equidistant, Stereographic};

#[test]
fn block_summaries_match_direct_integration_at_borders_and_keep_tiny_light() {
    use crate::model::Texel;
    use super::tile_sampling::TileSampler;
    use super::pixel_warp::content_bounds_tests::merge_image;

    let edge = crate::constants::TILE_EDGE_TEXELS;
    let mut texels: Vec<_> = (0..edge * edge).map(|index| {
        let alpha = (index % 19 + 1) as f32 / 21.0;
        Texel { rgb: [alpha * 0.2, alpha * 0.7, 0.0], opacity: alpha }
    }).collect();
    for dark_hole in [false, true] {
        if dark_hole {
            for row in 16..48 { for column in 16..48 { texels[row * edge + column] = Texel::default(); } }
        }
        let image = merge_image(texels.clone());
        let mut summarized = TileSampler::new(edge);
        let mut direct = TileSampler::new(edge);
        summarized.prepare(image.lit_rows(), image.filter_summary.as_ref());
        direct.prepare(image.lit_rows(), None);

        // Include different block alignments, rectangular boxes, clipped edges and selection thresholds.
        for index in 0..1000 {
            let position = [(index * 139 % 7100) as f64 / 100.0 - 3.0, (index * 503 % 7100) as f64 / 100.0 - 3.0];
            let footprint = [0.5 + (index % 97) as f64, 0.5 + (index % 83) as f64];
            let expected = direct.sample(image.texels(), position, footprint);
            let actual = summarized.sample(image.texels(), position, footprint);
            assert_eq!(actual.is_some(), expected.is_some());
            for (actual, expected) in actual.unwrap_or([0.0; 4]).into_iter().zip(expected.unwrap_or([0.0; 4])) {
                assert!((actual - expected).abs() <= 1e-6 * expected.abs().max(1e-30));
            }
        }
        if dark_hole {
            for epsilon in [0.0, 1e-12, 1e-9, 1e-6] {
                let expected = direct.sample(image.texels(), [32.0; 2], [32.0 + epsilon; 2]);
                let actual = summarized.sample(image.texels(), [32.0; 2], [32.0 + epsilon; 2]);
                if epsilon == 0.0 { assert_eq!(actual, None); }
                for (actual, expected) in actual.unwrap_or([0.0; 4]).into_iter().zip(expected.unwrap_or([0.0; 4])) {
                    assert!((actual - expected).abs() <= 1e-6 * expected.abs().max(1e-30));
                }
            }
        }
    }
}

#[test]
#[ignore = "single-thread release comparison of filtering summaries; synthetic tiles only"]
fn time_filtering_summaries() {
    use std::hint::black_box;
    use std::time::Instant;

    let date = crate::formulas::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
    let time = crate::formulas::utc_jd_to_time_ms(crate::formulas::datetime_to_julian_date(&date)).unwrap();
    let observer = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
    let earth = evaluate_earth_state(time);
    let mut data = Data::default();
    eprintln!("Filter summary header bytes={}, block bytes per image={}",
        std::mem::size_of::<Option<crate::model::TileFilterSummary>>(),
        (crate::constants::TILE_EDGE_TEXELS / crate::constants::TILE_FILTER_BLOCK_EDGE).pow(2) * 32);
    let selected: Vec<_> = (0..data.star_regions.len()).collect();

    for full in [false, true] {
        // Construction/replacement is outside steady-frame timing, with no copying of cached arrays per run.
        let preparation = Instant::now();
        for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, full); }
        eprintln!("Filtering fixture full={full}: build and merge {:?}", preparation.elapsed());
        let mut saved = vec![None; selected.len()];
        for (fov, projection, atmosphere) in [(180.0, Stereographic, false), (180.0, Stereographic, true),
            (351.6, Stereographic, false), (359.0, Stereographic, false), (360.0, Equidistant, false)]
        {
            let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, projection };
            let observation = observe(&observer, atmosphere, &view, time, earth, &mut StepTimes::default());
            let mut direct_times = Vec::new();
            let mut summary_times = Vec::new();
            for round in 0..6 {
                for enabled in if round % 2 == 0 { [false, true] } else { [true, false] } {
                    for (region, saved) in data.star_regions.iter_mut().zip(&mut saved) {
                        if region.merged_tile.filter_summary.is_some() != enabled {
                            std::mem::swap(&mut region.merged_tile.filter_summary, saved);
                        }
                    }
                    let start = Instant::now();
                    let stars = project_test_image(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
                    black_box(&stars);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if round > 0 {
                        if enabled { summary_times.push(elapsed); } else { direct_times.push(elapsed); }
                    }
                }
            }
            direct_times.sort_by(f64::total_cmp);
            summary_times.sort_by(f64::total_cmp);

            // The final round ends with direct sampling. Restore the same summaries for output comparison.
            let direct = project_test_image(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            for (region, saved) in data.star_regions.iter_mut().zip(&mut saved) {
                std::mem::swap(&mut region.merged_tile.filter_summary, saved);
            }
            let summarized = project_test_image(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            eprintln!("Filtering full={full} {projection:?} fov={fov} atmosphere={atmosphere}: direct {:.3} ms [{:.3}, {:.3}], summaries {:.3} ms [{:.3}, {:.3}], RGBA error {}",
                direct_times[2], direct_times[0], direct_times[4], summary_times[2], summary_times[0], summary_times[4], compare_images(&direct, &summarized));
        }
    }
}
