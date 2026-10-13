//! Adaptive forward projection using the real sky mapping. Each region starts as one rectangle; edge and
//! center checks select a finer uniform grid only for regions that need it. A uniform grid within a region
//! avoids T-junction cracks. Singular/nonconvex pieces trigger refinement rather than rejecting the region.
//! This is sampled error control with a hard work limit, not a proven global error bound. No screen rays or
//! polynomial patch fits are used. Coons, Hermite and the straight backend remain separately selectable.
use std::collections::HashMap;
use std::f64::consts::PI;
use crate::astro::Vector3;
use crate::constants::{ABERRATION_MARGIN, ADAPTIVE_WARP_ERROR_PIXELS, ADAPTIVE_WARP_MAX_SUBDIVISIONS, REFRACTION_MARGIN, TILE_EDGE_TEXELS};
use crate::model::{Data, ProjectionKind, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, prepare_camera, project_camera};
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::timing::StepTimes;
use super::patch_warp::{RegionPixels, Triangle};
use super::raster::Point;
use super::tile_sampling::TileSampler;
use super::tile_warp::{contains_antipode, is_convex};

/// Select adaptive stereographic geometry; equidistant retains its existing straight renderer.
#[allow(dead_code)] // backend selection is one call in projection/mod.rs
pub(super) fn project_adaptive_tiles(
    frame: &mut Frame, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes,
) {
    // Keep -e outside the scope of this prototype.
    if view.projection != ProjectionKind::Stereographic {
        super::tile_warp::project_whole_tiles(frame, data, selected, observation, view, times);
        return;
    }

    // Prepare one camera and reuse temporary allocations across regions in this frame.
    times.measure_detail("Star layer clear", || frame.stars_mut().fill(Texel::default()));
    let (width, height, sky) = (frame.width(), frame.height(), frame.sky());
    if width == 0 || height == 0 || sky.size < 2 { return; }
    let camera = times.measure_detail("Camera preparation", || prepare_camera(view));
    let mut samples = HashMap::new();
    let mut mesh = Vec::new();
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    let mut pixels = RegionPixels::default();
    let mut totals = MeshStats::default();
    let mut passes = times.detail_batch([
        "Tile geometry", "Tile corner projection", "Adaptive mesh preparation",
        "Tile filter preparation", "Tile rasterization", "Tile composition",
    ]);

    // Build directly projected meshes, then accumulate all pieces before compositing each complete region.
    for &region in &selected.regions {
        let image = &data.star_regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        let tile = passes.measure(0, || TileGeometry::new(region));
        let projector = TileProjector { tile, observation, camera, sky, half_fov: view.fov_degrees.to_radians() * 0.5 };
        samples.clear();
        passes.measure(1, || {
            for key in corner_keys(0, 0, grid_size()) { sample_vertex(&projector, &mut samples, key); }
        });
        let stats = passes.measure(2, || build_mesh(&projector, &mut samples, &mut mesh));
        totals.projections += stats.projections;
        totals.capped_cells += stats.capped_cells;
        totals.skipped_cells += stats.skipped_cells;
        totals.divisions = totals.divisions.max(stats.divisions);
        if mesh.is_empty() { continue; }
        passes.measure(3, || sampler.prepare(image.lit_rows()));
        passes.measure(4, || {
            pixels.prepare(&mesh, width, height, sky);
            for triangle in &mesh { pixels.draw_triangle(triangle, &sampler, image.texels()); }
        });
        passes.measure(5, || pixels.composite(frame.stars_mut(), width));
    }
    passes.publish(times);
    times.describe("Adaptive mesh preparation", || format!(
        "{} sky projections; maximum {}x{} cells per region; {} cells above target at limit; {} singular/folded cells omitted",
        totals.projections, totals.divisions, totals.divisions, totals.capped_cells, totals.skipped_cells,
    ));
}

struct TileProjector<'a> {
    tile: TileGeometry,
    observation: &'a Observation,
    camera: CartesianCamera,
    sky: SkyArea,
    half_fov: f64,
}

#[derive(Clone, Copy)]
struct Vertex {
    catalog: Vector3,
    apparent: Vector3,
    screen: Option<Point>,                 // None at a singular or nonfinite projected position
}

#[derive(Default, Debug)]
struct MeshStats {
    projections: usize,
    divisions: usize,
    capped_cells: usize,
    skipped_cells: usize,
}

fn grid_size() -> usize {
    // Integer dyadic coordinates allow exact sharing of vertices across all refinement levels.
    assert!(ADAPTIVE_WARP_MAX_SUBDIVISIONS.is_power_of_two());
    2 * ADAPTIVE_WARP_MAX_SUBDIVISIONS // leaves room for midpoint checks at the finest level
}

fn corner_keys(left: usize, top: usize, step: usize) -> [[usize; 2]; 4] {
    // Walk the rectangle boundary in texture order, independent of its eventual screen winding.
    [[left, top], [left + step, top], [left + step, top + step], [left, top + step]]
}

/// Reuse a projection by integer tile coordinate. The caller clears the map before changing region/view;
/// the map is frame-local scratch, with no reuse across frames or observers. Each coordinate is evaluated once.
fn sample_vertex(projector: &TileProjector<'_>, samples: &mut HashMap<[usize; 2], Vertex>, key: [usize; 2]) -> Vertex {
    // Convert the shared vertex coordinate into a position in the original cached tile.
    *samples.entry(key).or_insert_with(|| {
        let scale = TILE_EDGE_TEXELS as f64 / grid_size() as f64;
        let catalog = projector.tile.direction_of(key[0] as f64 * scale - 0.5, key[1] as f64 * scale - 0.5);
        let apparent = projector.observation.apparent_star_direction(catalog);
        let radius = (projector.sky.size as f64 - 1.0) * 0.5;
        let screen = project_camera(projector.camera, apparent).and_then(|point| {
            let screen = [projector.sky.x as f64 + radius + radius * point.x, projector.sky.y as f64 + radius - radius * point.y];
            screen.iter().all(|value| value.is_finite()).then_some(screen)
        });
        Vertex { catalog, apparent, screen }
    })
}

/// Reject only cells whose spherical cap cannot reach the view, allowing for all observer corrections.
/// A cap around the normalized tangent-plane center containing the four corners contains the whole cell.
fn is_outside_view(projector: &TileProjector<'_>, corners: [Vertex; 4], center: Vertex) -> bool {
    // The source cell is a small convex spherical rectangle; bound its radius before apparent corrections.
    let radius = corners.iter().map(|corner| {
        center.catalog.cross(corner.catalog).length().atan2(center.catalog.dot(corner.catalog))
    }).fold(0.0_f64, f64::max);
    let correction = ABERRATION_MARGIN + if projector.observation.atmosphere { REFRACTION_MARGIN } else { 0.0 };
    let reach = projector.half_fov + radius + correction;
    if reach >= PI { return false; }

    // Culling in angular space also discards pieces around the antipode without projecting a giant polygon.
    let rotated_center = projector.observation.to_horizon.apply(center.catalog);
    projector.camera.forward.dot(rotated_center) < reach.cos()
}

/// Produce a conforming grid for one region. Reuse previously projected vertices while doubling resolution.
/// Invalid cells are refined too; at the limit only those cells are omitted, preserving the rest of the tile.
fn build_mesh(projector: &TileProjector<'_>, samples: &mut HashMap<[usize; 2], Vertex>, mesh: &mut Vec<Triangle>) -> MeshStats {
    // Refine per region rather than mixing cell sizes, so neighboring pieces cannot disagree along an edge.
    let mut divisions = 1;
    loop {
        mesh.clear();
        let mut stats = MeshStats { divisions, ..MeshStats::default() };
        let mut refine = false;
        let step = grid_size() / divisions;
        for row in 0..divisions {
            for column in 0..divisions {
                let keys = corner_keys(column * step, row * step, step);
                let corners = keys.map(|key| sample_vertex(projector, samples, key));
                let center_key = [column * step + step / 2, row * step + step / 2];
                let center = sample_vertex(projector, samples, center_key);
                if is_outside_view(projector, corners, center) { continue; }

                // A piece crossing the projection singularity or folding cannot be drawn as two triangles.
                let finite = corners.iter().all(|corner| corner.screen.is_some());
                let screen = corners.map(|corner| corner.screen.unwrap_or([f64::NAN; 2]));
                if !finite || contains_antipode(corners.map(|corner| corner.apparent), projector.camera.forward) || !is_convex(screen) {
                    refine = true;
                    stats.skipped_cells += 1;
                    continue;
                }

                // Compare actual midpoint projections with interpolation along every edge and the diagonal.
                let mut error: f64 = 0.0;
                for (start, end) in [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)] {
                    let midpoint_key = [(keys[start][0] + keys[end][0]) / 2, (keys[start][1] + keys[end][1]) / 2];
                    let midpoint = sample_vertex(projector, samples, midpoint_key);
                    let deviation = match midpoint.screen {
                        Some(point) => (point[0] - (screen[start][0] + screen[end][0]) * 0.5)
                            .hypot(point[1] - (screen[start][1] + screen[end][1]) * 0.5),
                        None => f64::INFINITY,
                    };
                    error = error.max(deviation);
                }
                if !error.is_finite() {
                    refine = true;
                    stats.skipped_cells += 1;
                    continue;
                }
                if error > ADAPTIVE_WARP_ERROR_PIXELS {
                    refine = true;
                    stats.capped_cells += 1;
                }

                // Keep texture coordinates in the full original image; no sub-images are copied or redrawn.
                let texture = keys.map(|key| key.map(|value| value as f64 * TILE_EDGE_TEXELS as f64 / grid_size() as f64));
                let triangles = [[0, 1, 2], [0, 2, 3]].map(|indices| {
                    Triangle::new(indices.map(|index| screen[index]), indices.map(|index| texture[index]))
                });
                if let [Some(first), Some(second)] = triangles {
                    mesh.push(first);
                    mesh.push(second);
                } else {
                    refine = true;
                    stats.skipped_cells += 1;
                }
            }
        }
        if !refine || divisions == ADAPTIVE_WARP_MAX_SUBDIVISIONS {
            stats.projections = samples.len();
            return stats;
        }
        divisions *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, Observer};
    use crate::constants::GRID_DEPTH;
    use crate::model::{ViewCenter, hash_direction, region_caps};
    use crate::observation::observe;

    fn fixture(fov: f64, atmosphere: bool) -> (View, Observation, CartesianCamera, SkyArea) {
        // Reproduce the screenshot's date, observer, orientation and frame dimensions without loading stars.
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, ..View::default() };
        let date = crate::astro::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
        let time = FrameTime::from_utc(crate::astro::datetime_to_julian_date(&date));
        let site = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
        let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
        (view, observation, prepare_camera(&view), SkyArea::centred(1917, 1116))
    }

    fn projector<'a>(region: usize, observation: &'a Observation, camera: CartesianCamera, sky: SkyArea, view: &View) -> TileProjector<'a> {
        // One region's mapping; samples belonging to a different projector must not be reused.
        TileProjector { tile: TileGeometry::new(region), observation, camera, sky, half_fov: view.fov_degrees.to_radians() * 0.5 }
    }

    fn exact_position(projector: &TileProjector<'_>, uv: Point) -> Option<Point> {
        // Independent test oracle at arbitrary texture positions, including positions not used by the mesh.
        let catalog = projector.tile.direction_of(uv[0] - 0.5, uv[1] - 0.5);
        let apparent = projector.observation.apparent_star_direction(catalog);
        let point = project_camera(projector.camera, apparent)?;
        let radius = (projector.sky.size as f64 - 1.0) * 0.5;
        Some([projector.sky.x as f64 + radius + point.x * radius, projector.sky.y as f64 + radius - point.y * radius])
    }

    fn is_visible(point: Point, sky: SkyArea) -> bool {
        let radius = (sky.size as f64 - 1.0) * 0.5;
        (point[0] - sky.x as f64 - radius).hypot(point[1] - sky.y as f64 - radius) <= radius
    }

    #[test]
    fn flat_regions_stay_coarse_and_vertices_are_reused_only_inside_one_region() {
        // A nearly central tile at a wide view needs few pieces; repeated lookups reuse its exact vertices.
        let (view, observation, camera, sky) = fixture(351.6, false);
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let projector = projector(region, &observation, camera, sky, &view);
        let mut samples = HashMap::new();
        let mut mesh = Vec::new();
        let stats = build_mesh(&projector, &mut samples, &mut mesh);
        assert_eq!(stats.divisions, 1);
        assert_eq!(mesh.len(), 2);
        assert_eq!(samples.len(), 9);
        let key = [grid_size() / 2; 2];
        let first = sample_vertex(&projector, &mut samples, key).screen;
        assert_eq!(sample_vertex(&projector, &mut samples, key).screen, first);
        assert_eq!(samples.len(), 9);
        samples.clear();
        assert!(samples.is_empty());
    }

    #[test]
    fn extreme_view_keeps_visible_parts_and_tracks_the_actual_projection() {
        // Audit all 6144 region geometries at the failing zoom level. Probe both the tile domain for holes
        // and triangle interiors for interpolation error, independently of the refinement midpoints.
        for atmosphere in [false, true] {
            let (view, observation, camera, sky) = fixture(351.6, atmosphere);
            let mut samples = HashMap::new();
            let mut mesh = Vec::new();
            let (mut worst, mut checked, mut rejected, mut capped, mut refined, mut lost_regions) = (0.0_f64, 0, 0, 0, 0, 0);
            for region in 0..region_caps().len() {
                samples.clear();
                let projector = projector(region, &observation, camera, sky, &view);
                let stats = build_mesh(&projector, &mut samples, &mut mesh);
                rejected += stats.skipped_cells;
                capped += stats.capped_cells;
                refined += usize::from(stats.divisions > 1);
                assert!(stats.divisions <= ADAPTIVE_WARP_MAX_SUBDIVISIONS);
                assert!(stats.projections <= (grid_size() + 1).pow(2));
                assert!(mesh.len() <= 2 * ADAPTIVE_WARP_MAX_SUBDIVISIONS.pow(2));

                // If a region has independently sampled visible positions it must retain a renderable piece.
                let visible_region = [0.1, 0.5, 0.9].into_iter().any(|u| [0.1, 0.5, 0.9].into_iter().any(|v| {
                    exact_position(&projector, [u * TILE_EDGE_TEXELS as f64, v * TILE_EDGE_TEXELS as f64]).is_some_and(|point| is_visible(point, sky))
                }));
                if visible_region && mesh.is_empty() { lost_regions += 1; }

                // Check barycenters and off-center locations, rather than just rechecking the tested midpoints.
                for triangle in &mesh {
                    for weights in [[1.0 / 3.0; 3], [0.6, 0.2, 0.2], [0.2, 0.6, 0.2], [0.2, 0.2, 0.6]] {
                        let interpolate = |points: [Point; 3]| -> Point {
                            std::array::from_fn(|axis| (0..3).map(|index| weights[index] * points[index][axis]).sum())
                        };
                        let uv = interpolate(triangle.texture);
                        let expected = exact_position(&projector, uv).unwrap();
                        if !is_visible(expected, sky) { continue; }
                        let actual = interpolate(triangle.screen);
                        worst = worst.max((expected[0] - actual[0]).hypot(expected[1] - actual[1]));
                        checked += 1;
                    }
                }
            }
            eprintln!("Adaptive 351.6deg atmosphere={atmosphere}: refined_regions={refined}, skipped_cells={rejected}, capped_cells={capped}, lost_visible_regions={lost_regions}, probes={checked}, worst_error={worst:.6}px");
            assert!(checked > 0 && refined > 0);
            assert_eq!(lost_regions, 0);
            assert!(worst < 1.0, "{worst}px interpolation error at the reported failure view");
        }
    }

    #[test]
    fn mesh_edges_match_and_opaque_pieces_do_not_overlap_or_leave_seams() {
        // A tile near the outer rim requires refinement. Shared grid edges must be exactly equal in screen space.
        let (view, observation, camera, sky) = fixture(351.6, false);
        let angle = 175_f64.to_radians();
        let direction = observation.to_catalog.apply(camera.forward * angle.cos() + camera.up * angle.sin());
        let region = hash_direction(GRID_DEPTH, direction);
        let projector = projector(region, &observation, camera, sky, &view);
        let mut samples = HashMap::new();
        let mut mesh = Vec::new();
        let stats = build_mesh(&projector, &mut samples, &mut mesh);
        assert!(stats.divisions > 1);
        let mut positions = HashMap::new();
        for triangle in &mesh {
            for (texture, screen) in triangle.texture.into_iter().zip(triangle.screen) {
                let key = texture.map(f64::to_bits);
                if let Some(previous) = positions.insert(key, screen) { assert_eq!(previous, screen); }
            }
        }

        // The real renderer must keep channels bounded when every texel is opaque.
        let mut data = Data::default();
        data.star_regions[region].merged_tile.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        data.star_regions[region].merged_tile.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: false };
        let mut frame = Frame::square(201);
        project_adaptive_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
        assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));
        project_adaptive_tiles(&mut frame, &data, &Selected { regions: vec![], whole_sky: false }, &observation, &view, &mut StepTimes::default());
        assert!(frame.stars().iter().all(|texel| texel.is_empty()));

        // The unchanged -e branch still renders byte-for-byte like the straight backend.
        let equidistant = View { projection: ProjectionKind::Equidistant, ..view };
        project_adaptive_tiles(&mut frame, &data, &selected, &observation, &equidistant, &mut StepTimes::default());
        let expected = frame.stars().to_vec();
        super::super::tile_warp::project_whole_tiles(&mut frame, &data, &selected, &observation, &equidistant, &mut StepTimes::default());
        assert_eq!(frame.stars(), expected);
    }
    #[test]
    #[ignore = "timing only; uses embedded BSC5 and synthetic dense tiles, never external datasets"]
    fn time_adaptive_mapping() {
        // Use one prepared catalog and the screenshot's camera; only projection is timed.
        let mut report = crate::progress::Report::quiet();
        let catalog = crate::catalog::load_embedded_catalog(&mut report).unwrap();
        let prepared = crate::sky::prepare_owned_catalog(catalog, &mut report).unwrap();
        let original = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        for fov in [180.0, 351.6] {
            let (view, observation, _, _) = fixture(fov, false);
            let mut data = original.clone();
            let selected = crate::selection::select_regions(&data.star_regions, &observation, &mut StepTimes::default());
            let settings = crate::model::RenderOptions { magnitude_threshold: 9.0, unicode: true, braille: false, color: true,
                constellations: false, grid: false, dynamic_names: false };
            crate::regions::refresh_regions(&mut data, &selected, observation.time, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::new(1917, 1116, SkyArea::centred(1917, 1116), (10, 20));
            for dense in [false, true] {
                if dense {
                    for &region in &selected.regions {
                        let image = &mut data.star_regions[region].merged_tile;
                        image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS.pow(2)];
                        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
                    }
                }
                let mut elapsed = 0.0;
                for run in 0..4 {
                    let start = std::time::Instant::now();
                    project_adaptive_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                    if run > 0 { elapsed += start.elapsed().as_secs_f64(); }
                    std::hint::black_box(frame.stars());
                }
                eprintln!("Adaptive fov={fov}, dense={dense}, regions={}: {:.3}ms; three frames after warmup", selected.regions.len(), elapsed * 1000.0 / 3.0);
                let mut detail = StepTimes::with_trace(true);
                project_adaptive_tiles(&mut frame, &data, &selected, &observation, &view, &mut detail);
                for step in detail.steps().iter().filter(|step| matches!(step.name, "Adaptive mesh preparation" | "Tile filter preparation" | "Tile rasterization" | "Tile composition")) {
                    eprintln!("  {}: {:.3}ms (separate traced frame)", step.name, step.average_seconds * 1000.0);
                }
            }
        }
    }

}
