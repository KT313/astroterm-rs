//! Coverage and performance checks for image-derived candidate bounds, beside the inverse warp oracle.
use super::*;
use super::tests::{facing, observation_at};
use crate::constants::{GRID_DEPTH, SPEED_GROUP_COUNT};
use crate::model::{GroupTileKey, RegionTileCache, StarGroup, hash_direction};
use ProjectionKind::{Equidistant, Stereographic};

pub(in crate::projection) fn merge_image(texels: Vec<Texel>) -> RegionTileCache {
    // Synthetic group contents still go through the real merge and derived-bound calculation.
    let mut groups: [StarGroup; SPEED_GROUP_COUNT] = std::array::from_fn(|_| StarGroup::default());
    groups[0].tile.key = Some(GroupTileKey { star_count: 1, ..GroupTileKey::EMPTY });
    groups[0].tile.tile.texels = texels;
    let mut image = RegionTileCache::default();
    image.merge(&groups);
    image
}

pub(in crate::projection) fn build_dense_image(region: usize, fill_square: bool) -> RegionTileCache {
    // Fill the region's own cell, leaving its tile padding empty, or fill the whole square for a stress case.
    let geometry = TileGeometry::new(region);
    let edge = TILE_EDGE_TEXELS;
    let mut texels = vec![Texel::default(); edge * edge];
    for (index, texel) in texels.iter_mut().enumerate() {
        let direction = geometry.direction_of((index % edge) as f64, (index / edge) as f64);
        if !fill_square && hash_direction(GRID_DEPTH, direction) != region { continue; }
        let opacity = 0.1 + (index % 37) as f32 / 50.0;
        *texel = Texel { rgb: [opacity * 0.3, opacity * 0.5, opacity * 0.7], opacity };
    }
    merge_image(texels)
}

fn count_candidates(mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>) -> usize {
    // Count exactly the pixels that reach the mapping loop, excluding rows the sparse masks skip.
    let mut count = 0;
    let (top, bottom) = mapping.rows();
    for row in top..=bottom {
        for (first, last) in mapping.columns_of(row).into_iter().flatten() {
            let Some((first_tile_row, last_tile_row)) = mapping.tile_rows_of(row, first, last) else { continue; };
            if sampler.any_light_in_rows(first_tile_row, last_tile_row) {
                count += last - first + 1;
            }
        }
    }
    count
}

fn compare_with_full_disc(region: usize, image: &RegionTileCache, observation: &Observation, view: &View, size: usize) {
    // Build the reference transform without consulting any candidate bound, even for off-screen regions.
    let geometry = TileGeometry::new(region);
    let screen = Screen::new(prepare_camera(view), size);
    let (east, north, centre) = (geometry.east(), geometry.north(), geometry.center());
    let axes = Matrix3([[east.x, east.y, east.z], [north.x, north.y, north.z], [centre.x, centre.y, centre.z]]);
    let to_tile = axes.compose(observation.to_catalog);
    let aberration_horizon = observation.to_horizon.apply(observation.observer.velocity * (1.0 / LIGHT_SPEED_AU_DAY));
    let reference = RegionMapping {
        tile_axes: axes,
        to_tile,
        aberration: to_tile.apply(aberration_horizon),
        aberration_horizon,
        density: geometry.density(),
        atmosphere: observation.atmosphere,
        screen: &screen,
        image: Image::WholeDisc,
    };
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    sampler.prepare(image.lit_rows(), image.filter_summary.as_ref());
    let mut actual = vec![Texel::default(); size * size];
    if let Some(mapping) = RegionMapping::new(&geometry, observation, &screen, image.content_radius) {
        draw_region(&mut actual, &mapping, &sampler, image.texels());
    }

    // Scan the whole disc directly, bypassing bounds, stepped rays, row skips and early footprint rejection.
    for row in 0..size {
        for column in 0..size {
            let squared_distance = (column as f64 - screen.radius).powi(2) + (row as f64 - screen.radius).powi(2);
            if squared_distance > screen.radius * screen.radius + 1e-9 { continue; }
            let (numerator, spread, factor) = screen.ray(column as f64, row as f64);
            let expected = reference.tile_of(numerator, spread, factor).and_then(|(position, footprint)| {
                sampler.sample(image.texels(), position.map(|value| value + 0.5), [footprint; 2])
            }).unwrap_or([0.0; 4]);
            let pixel = actual[row * size + column];
            for (value, expected) in pixel.rgb.into_iter().chain([pixel.opacity]).zip(expected) {
                assert!((value - expected).abs() < 1e-6,
                    "{:?} fov={} atmosphere={} region={region} radius={:?} pixel=({column},{row}): {value} != {expected}",
                    view.projection, view.fov_degrees, observation.atmosphere, image.content_radius);
            }
        }
    }
}

#[test]
fn missing_invalid_and_refracted_bounds_keep_the_full_tile() {
    let full_radius = (TileGeometry::middle() + 1.0) * SQRT_2;
    for radius in [None, Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY), Some(full_radius + 1.0)] {
        assert_eq!(select_content_radius(radius, false), full_radius);
    }
    assert_eq!(select_content_radius(Some(2.0), false), 2.0);
    assert_eq!(select_content_radius(Some(2.0), true), full_radius);
}

#[test]
fn tight_bounds_retain_dense_cells_islands_and_outer_edges() {
    let region = hash_direction(GRID_DEPTH, Vector3 { x: 0.3, y: 0.8, z: -0.2 });
    let geometry = TileGeometry::new(region);
    let edge = TILE_EDGE_TEXELS;

    // Use both filled regions and isolated light, including every edge and disconnected islands.
    let mut images = vec![build_dense_image(region, false), build_dense_image(region, true)];
    for positions in [vec![(31, 31)], vec![(0, 0)], vec![(63, 0)], vec![(0, 63)], vec![(63, 63)],
        vec![(31, 0), (0, 31), (63, 31), (31, 63)], vec![(15, 17), (46, 42)]]
    {
        let mut texels = vec![Texel::default(); edge * edge];
        for (column, row) in positions {
            texels[row * edge + column] = Texel { rgb: [0.25, 0.5, 0.125], opacity: 0.75 };
        }
        images.push(merge_image(texels));
    }

    // Position the region at the centre, near the rim and behind the view; vary subpixel alignment too.
    for projection in [Stereographic, Equidistant] {
        for fov in [10.0, 60.0, 180.0, 300.0, if projection == Equidistant { 360.0 } else { 359.0 }] {
            let (_, initial) = observation_at(fov, false, facing(0.0, 0.0), projection);
            let apparent = initial.apparent_star_direction(geometry.center());
            let tangent = apparent.cross(Vector3 { x: 0.0, y: 0.0, z: 1.0 }).normalized();
            for angle in [0.001_f64, fov.to_radians() * 0.45, PI - 0.001] {
                let camera_direction = apparent * angle.cos() + tangent * angle.sin();
                let centre = Horizontal::from_vector(camera_direction);
                let (view, observation) = observation_at(fov, false, facing(centre.azimuth, centre.altitude), projection);
                for image in &images {
                    compare_with_full_disc(region, image, &observation, &view, 301);
                }
            }
        }
    }
}

#[test]
fn tight_support_crossing_the_antipode_keeps_all_partial_coverage() {
    // Move the support circle through the point behind the camera: interior, half-plane, then exterior.
    for direction in [Vector3 { x: 1.0, y: 0.99, z: 0.99 }, Vector3 { x: 0.01, y: 0.0, z: 1.0 }] {
        let region = hash_direction(GRID_DEPTH, direction);
        let geometry = TileGeometry::new(region);
        let image = build_dense_image(region, false);
        let angular_reach = 2.0 * (image.content_radius.unwrap() / (2.0 * geometry.density())).atan();
        let (_, initial) = observation_at(359.0, false, facing(0.0, 0.0), Stereographic);
        let apparent = initial.apparent_star_direction(geometry.center());
        let tangent = apparent.cross(Vector3 { x: 0.0, y: 0.0, z: 1.0 }).normalized();
        for offset in [-1e-5, 0.0, 1e-5] {
            let angle = PI - angular_reach + offset;
            let camera_direction = apparent * angle.cos() + tangent * angle.sin();
            let centre = Horizontal::from_vector(camera_direction);
            let (view, observation) = observation_at(359.0, false, facing(centre.azimuth, centre.altitude), Stereographic);
            compare_with_full_disc(region, &image, &observation, &view, 301);
        }
    }
}

#[test]
fn content_bounds_reduce_work_without_changing_the_refracted_image() {
    let (view, observation) = observation_at(180.0, false, facing(334_f64.to_radians(), 0.0), Stereographic);
    let region = hash_direction(GRID_DEPTH, observation.patch.center);
    let image = build_dense_image(region, false);
    let geometry = TileGeometry::new(region);
    let screen = Screen::new(prepare_camera(&view), 1116);
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    sampler.prepare(image.lit_rows(), image.filter_summary.as_ref());
    let original = RegionMapping::new(&geometry, &observation, &screen, None).unwrap();
    let tight = RegionMapping::new(&geometry, &observation, &screen, image.content_radius).unwrap();
    assert!(count_candidates(&tight, &sampler) < count_candidates(&original, &sampler));
    let mut original_pixels = vec![Texel::default(); screen.size * screen.size];
    let mut tight_pixels = original_pixels.clone();
    draw_region(&mut original_pixels, &original, &sampler, image.texels());
    draw_region(&mut tight_pixels, &tight, &sampler, image.texels());
    let original_samples = original_pixels.iter().filter(|pixel| !pixel.is_empty()).count();
    let tight_samples = tight_pixels.iter().filter(|pixel| !pixel.is_empty()).count();
    assert_eq!(tight_samples, original_samples);
    assert!(tight_samples > 0);

    // The failed assessment case used a facing view at the horizon with refraction enabled.
    let (view, observation) = observation_at(180.0, true, facing(334_f64.to_radians(), 0.0), Stereographic);
    let screen = Screen::new(prepare_camera(&view), 1116);
    for region in (0..crate::constants::CELL_COUNT).step_by(17) {
        let geometry = TileGeometry::new(region);
        let image = build_dense_image(region, false);
        let original = RegionMapping::new(&geometry, &observation, &screen, None);
        let cached = RegionMapping::new(&geometry, &observation, &screen, image.content_radius);
        assert_eq!(original.map(|mapping| mapping.image), cached.map(|mapping| mapping.image));
    }
}

#[test]
#[ignore = "release timing only; synthetic tiles, no external datasets"]
fn time_cached_content_bounds() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut data = Data::default();
    let selected: Vec<_> = (0..data.star_regions.len()).collect();
    for full in [false, true] {
        // Build image summaries once, just as an infrequent group-image change would do.
        let preparation = Instant::now();
        for &region in &selected {
            data.star_regions[region].merged_tile = build_dense_image(region, full);
        }
        let radii: Vec<_> = data.star_regions.iter().map(|region| region.merged_tile.content_radius).collect();
        eprintln!("Content-bound fixture full={full}: preparation {:?}", preparation.elapsed());

        for (fov, projection, atmosphere) in [(60.0, Stereographic, false), (180.0, Stereographic, false),
            (180.0, Stereographic, true), (351.6, Stereographic, false), (360.0, Equidistant, false)]
        {
            let (view, observation) = observation_at(fov, atmosphere, facing(334_f64.to_radians(), 0.0), projection);
            let mut original_times = Vec::new();
            let mut cached_times = Vec::new();

            // Alternate the two paths, excluding the first pair and including output allocation in timing.
            for round in 0..6 {
                for cached in if round % 2 == 0 { [false, true] } else { [true, false] } {
                    for (region, &radius) in data.star_regions.iter_mut().zip(&radii) {
                        region.merged_tile.content_radius = if cached { radius } else { None };
                    }
                    let start = Instant::now();
                    let stars = project_tiles_by_pixel(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
                    black_box(&stars);
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if round == 0 { continue; }
                    if cached { cached_times.push(elapsed); } else { original_times.push(elapsed); }
                }
            }
            original_times.sort_by(f64::total_cmp);
            cached_times.sort_by(f64::total_cmp);
            eprintln!("Content bounds full={full} {projection:?} fov={fov} atmosphere={atmosphere}: original {:.3} ms, cached {:.3} ms",
                original_times[2], cached_times[2]);

            // Compare complete compositions outside the timed loop, including overlapping regions.
            for region in &mut data.star_regions { region.merged_tile.content_radius = None; }
            let original = project_tiles_by_pixel(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            for (region, &radius) in data.star_regions.iter_mut().zip(&radii) { region.merged_tile.content_radius = radius; }
            let cached = project_tiles_by_pixel(1116, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            let mut greatest_error = 0.0_f32;
            for (original, cached) in original.iter().zip(&cached) {
                for (original, cached) in original.rgb.into_iter().chain([original.opacity]).zip(cached.rgb.into_iter().chain([cached.opacity])) {
                    greatest_error = greatest_error.max((original - cached).abs());
                }
            }
            assert!(greatest_error < 1e-6, "full={full} {projection:?} fov={fov} atmosphere={atmosphere}: error={greatest_error}");
            eprintln!("  maximum RGBA difference: {greatest_error}");
        }
    }
}
