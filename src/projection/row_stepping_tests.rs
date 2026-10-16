//! Numerical and rendering comparisons with the pre-transformation row path.
use super::*;
use super::content_bounds_tests::build_dense_image;
use super::tests::{facing, observation_at};
use crate::model::{ViewCenter, region_caps};
use ProjectionKind::{Equidistant, Stereographic};

// Preserve the phase-1 loop as a test oracle, including its separate screen stepping and per-pixel transform.
fn draw_reference_region(stars: &mut [Texel], mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>, texels: &[Texel]) {
    let screen = mapping.screen;
    let (top, bottom) = mapping.rows();
    for row in top..=bottom {
        for (first, last) in mapping.columns_of(row).into_iter().flatten() {
            let Some((first_tile_row, last_tile_row)) = mapping.tile_rows_of(row, first, last) else { continue; };
            if !sampler.any_light_in_rows(first_tile_row, last_tile_row) { continue; }
            let pixels = &mut stars[row * screen.size..][first..=last];
            let mut rays = RowRays::new(screen, first as f64, row as f64);
            for texel in pixels {
                let (numerator, spread, factor) = rays.current(screen);
                rays.advance();
                let Some((position, footprint)) = mapping.tile_of_overlapping_footprint(numerator, spread, factor) else { continue; };
                if footprint <= 3.0 && !sampler.may_hold_light_near(position) { continue; }
                let sample_position = position.map(|value| value + 0.5);                  // integer texel centres become half-integer centres in the sampler's edge coordinates
                let Some(rgba) = sampler.sample(texels, sample_position, [footprint, footprint]) else { continue; };
                blend_star(texel, [rgba[0], rgba[1], rgba[2]], rgba[3]);
                texel.rgb = texel.rgb.map(|value| value.min(1.0));                          // absorb last-bit rounding of the blend at opaque pixels
                texel.opacity = texel.opacity.min(1.0);
            }
        }
    }
}

fn project_reference_tiles(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View) -> Vec<Texel> {
    // Match production setup and timing overhead, changing only the row drawing function.
    let mut times = StepTimes::default();
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size * size];
    if size < 2 { return stars; }
    times.measure("Projection.Camera preparation");
    let screen = Screen::new(prepare_camera(view), size);
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);

    // Both paths use the same phase-1 content bounds, selected order and source tiles.
    for &region in selected {
        let image = &regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(region);
        times.measure("Projection.Region mapping");
        let Some(mapping) = RegionMapping::new(&geometry, observation, &screen, image.content_radius) else { continue; };
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows(), image.filter_summary.as_ref());
        times.measure("Projection.Tile rasterization");
        draw_reference_region(&mut stars, &mapping, &sampler, image.texels());
    }
    times.measure("Projection");
    stars
}

/// Exercise the live unshared kernel explicitly, so later shared-screen changes do not alter this baseline.
pub(in crate::projection) fn project_unshared_tiles(size: usize, regions: &[Region], selected: &[usize], observation: &Observation, view: &View) -> Vec<Texel> {
    let mut times = StepTimes::default();
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size * size];
    if size < 2 { return stars; }
    times.measure("Projection.Camera preparation");
    let screen = Screen::new(prepare_camera(view), size);
    draw_unshared_tiles(&mut stars, &screen, regions, selected, observation, &mut times);
    times.measure("Projection");
    stars
}

pub(in crate::projection) fn compare_images(actual: &[Texel], expected: &[Texel]) -> f32 {
    assert_eq!(actual.len(), expected.len());
    let mut greatest_error = 0.0_f32;
    for (actual, expected) in actual.iter().zip(expected) {
        for (actual, expected) in actual.rgb.into_iter().chain([actual.opacity]).zip(expected.rgb.into_iter().chain([expected.opacity])) {
            assert!(actual.is_finite() && expected.is_finite());
            greatest_error = greatest_error.max((actual - expected).abs());
        }
    }
    assert!(greatest_error < 1e-6, "maximum RGBA difference: {greatest_error}");
    greatest_error
}

#[test]
fn transformed_values_and_footprints_match_direct_evaluation_along_long_rows() {
    let mut samples_checked = 0;
    let mut longest_span = 0;

    // Include wide viewports and both ordinary and exterior-circle spans at extreme fields of view.
    for size in [301, 1116, 2160, 8192] {
        for fov in [1.0, 10.0, 60.0, 180.0, 300.0, 351.6, 359.0] {
            let (view, observation) = observation_at(fov, false, facing(334_f64.to_radians(), 0.1), Stereographic);
            let screen = Screen::new(prepare_camera(&view), size);
            let mut regions: Vec<_> = (0..region_caps().len()).step_by(97).collect();
            regions.push(crate::model::hash_direction(crate::constants::GRID_DEPTH, observation.patch.center));
            regions.push(crate::model::hash_direction(crate::constants::GRID_DEPTH, -observation.patch.center));
            for region in regions {
                let tile = TileGeometry::new(region);
                let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue; };
                let (top, bottom) = mapping.rows();
                if top > bottom { continue; }
                for row in [top, top + (bottom - top) / 2, bottom] {
                    for (first, last) in mapping.columns_of(row).into_iter().flatten() {
                        longest_span = longest_span.max(last - first + 1);
                        let mut transformed = TransformedRow::new(&mapping, first as f64, row as f64);
                        for column in first..=last {
                            if (column - first).is_multiple_of(STEREOGRAPHIC_ROW_REBASE_PIXELS) {
                                transformed = TransformedRow::new(&mapping, column as f64, row as f64);
                            }
                            let (numerator, spread, factor) = screen.ray(column as f64, row as f64);
                            let expected_direction = mapping.to_tile.apply(numerator) - mapping.aberration * spread;
                            let expected_length = spread - numerator.dot(mapping.aberration_horizon);
                            let scale = spread.max(1.0);
                            assert!((transformed.direction - expected_direction).length() < 1e-7 * scale,
                                "size={size} fov={fov} region={region} ({column},{row}): direction drift");
                            assert!((transformed.length - expected_length).abs() < 1e-7 * scale);
                            assert!((transformed.spread - spread).abs() < 1e-7 * scale);

                            // Check actual accepted coordinates and footprint, not only polynomial coefficients.
                            let actual = transformed.project_footprint(&mapping);
                            let expected = mapping.tile_of_overlapping_footprint(numerator, spread, factor);
                            match (actual, expected) {
                                (Some((actual, width)), Some((expected, expected_width))) => {
                                    let tolerance = 1e-6 * expected_width.max(1.0);
                                    assert!((actual[0] - expected[0]).abs() < tolerance && (actual[1] - expected[1]).abs() < tolerance,
                                        "size={size} fov={fov} region={region} ({column},{row}): {actual:?} != {expected:?}");
                                    assert!((width - expected_width).abs() < tolerance);
                                    samples_checked += 1;
                                }
                                (None, None) => {}
                                mismatch => panic!("size={size} fov={fov} region={region} ({column},{row}): {mismatch:?}"),
                            }
                            transformed.advance();
                        }
                    }
                }
            }
        }
    }
    assert!(samples_checked > 1000, "{samples_checked}");
    assert!(longest_span > 4000, "{longest_span}");
}

#[test]
fn transformed_rows_preserve_composition_and_general_modes_are_identical() {
    let mut data = Data::default();
    let selected: Vec<_> = (0..data.star_regions.len()).step_by(13).collect();
    for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, true); }

    // Interleave mode changes and views, including refraction, the equidistant rim and near-singular stereo.
    for (fov, projection, atmosphere, centre) in [
        (60.0, Stereographic, false, facing(5.83, 0.0)),
        (180.0, Stereographic, true, ViewCenter::Zenith),
        (351.6, Stereographic, false, facing(5.83, 0.0)),
        (360.0, Equidistant, false, facing(5.83, 0.0)),
        (359.0, Stereographic, true, facing(5.83, 0.0)),
        (180.0, Stereographic, false, ViewCenter::Zenith),
        (360.0, Equidistant, true, facing(5.83, 0.0)),
    ] {
        let (view, observation) = observation_at(fov, atmosphere, centre, projection);
        for size in [0, 1, 301] {
            let expected = project_reference_tiles(size, &data.star_regions, &selected, &observation, &view);
            let actual = project_unshared_tiles(size, &data.star_regions, &selected, &observation, &view);
            let error = compare_images(&actual, &expected);
            if atmosphere || projection == Equidistant { assert_eq!(error, 0.0); }
        }
    }
}

#[test]
#[ignore = "release timing only; paired phase-1 and transformed paths, no external datasets"]
fn time_transformed_rows_with_content_bounds() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut data = Data::default();
    let selected: Vec<_> = (0..data.star_regions.len()).collect();
    for full in [false, true] {
        for &region in &selected { data.star_regions[region].merged_tile = build_dense_image(region, full); }
        for (fov, projection, atmosphere) in [(60.0, Stereographic, false), (180.0, Stereographic, false),
            (180.0, Stereographic, true), (351.6, Stereographic, false), (360.0, Equidistant, false)]
        {
            let (view, observation) = observation_at(fov, atmosphere, facing(334_f64.to_radians(), 0.0), projection);
            let mut reference_times = Vec::new();
            let mut transformed_times = Vec::new();

            // Alternate paths, warming both before recording five samples of each complete projection.
            for round in 0..6 {
                for transformed in if round % 2 == 0 { [false, true] } else { [true, false] } {
                    let start = Instant::now();
                    let stars = if transformed {
                        project_unshared_tiles(1116, &data.star_regions, &selected, &observation, &view)
                    } else {
                        project_reference_tiles(1116, &data.star_regions, &selected, &observation, &view)
                    };
                    black_box(&stars);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if round == 0 { continue; }
                    if transformed { transformed_times.push(elapsed); } else { reference_times.push(elapsed); }
                }
            }
            reference_times.sort_by(f64::total_cmp);
            transformed_times.sort_by(f64::total_cmp);

            // Compare full-frame output outside timing, with both paths using the same cached content bounds.
            let expected = project_reference_tiles(1116, &data.star_regions, &selected, &observation, &view);
            let actual = project_unshared_tiles(1116, &data.star_regions, &selected, &observation, &view);
            let error = compare_images(&actual, &expected);
            if atmosphere || projection == Equidistant { assert_eq!(error, 0.0); }
            eprintln!("Transformed rows full={full} {projection:?} fov={fov} atmosphere={atmosphere}: phase1 {:.3} ms, transformed {:.3} ms, maximum RGBA difference {error}",
                reference_times[2], transformed_times[2]);
        }
    }
}
