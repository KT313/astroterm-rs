//! Adaptive forward projection of the equidistant view (the stereographic view pulls pixels in `pixel_warp`).
//! Each region starts as one rectangle; edge and center checks select a finer uniform grid only for regions
//! that need it. A uniform grid within a region avoids T-junction cracks. Singular/nonconvex pieces trigger
//! refinement rather than rejecting the region. This is sampled error control with a hard work limit, not a
//! proven global error bound.
use std::collections::HashMap;
use std::f64::consts::PI;
use crate::astro::Vector3;
use crate::constants::{ABERRATION_MARGIN, ADAPTIVE_WARP_ERROR_PIXELS, ADAPTIVE_WARP_MAX_SUBDIVISIONS, REFRACTION_MARGIN, TILE_EDGE_TEXELS};
use crate::model::{Data, ProjectionKind, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, prepare_camera, project_camera};
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::time::StepTimes;
use super::mesh::{Point, contains_antipode, is_convex};
use super::patch_warp::{RegionPixels, Triangle};
use super::tile_sampling::TileSampler;

/// Adapt tile meshes using the camera formula of the view.
pub(super) fn project_adaptive_tiles(
    size: usize,
    data: &Data,
    selected: &Selected,
    observation: &Observation,
    view: &View,
    times: &mut StepTimes,
) -> Vec<Texel> {
    // Allocate an empty square for this projection; no terminal margins are involved.
    times.measure("Projection.Star buffer allocation");
    let mut stars = vec![Texel::default(); size.checked_mul(size).expect("drawing area pixel count overflow")];
    if size < 2 {
        return stars;
    }

    // Prepare the camera and reuse the temporary mesh and drawing buffers across regions.
    times.measure("Projection.Camera preparation");
    let camera = prepare_camera(view);
    let mut vertices = HashMap::new();
    let mut triangles = Vec::new();
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    let mut pixels = RegionPixels::default();
    let mut totals = MeshStats::default();

    for &region in &selected.regions {
        // Skip cached region images that contain no light.
        times.measure("Projection.Tile empty skip");
        let tile = &data.star_regions[region].merged_tile;
        if tile.is_empty() || tile.lit_rows().iter().all(|&row| row == 0) {
            continue;
        }

        // Build the region's screen mapping and count its results, including omitted cells.
        let stats = build_region_mesh(region, observation, camera, size, view, &mut vertices, &mut triangles, times);
        totals.add_region(&stats);
        if triangles.is_empty() {
            continue;
        }

        // Draw the cached tile through that mapping and combine it with the square image.
        draw_region_mesh(&mut stars, size, tile, &triangles, &mut sampler, &mut pixels, times);
    }

    // Attach the combined mesh statistics to this frame's diagnostic trace.
    totals.record_trace(times);
    stars
}

/// Build one region's projected triangles, reusing storage from the preceding region.
#[allow(clippy::too_many_arguments)]
fn build_region_mesh(
    region: usize,
    observation: &Observation,
    camera: CartesianCamera,
    size: usize,
    view: &View,
    vertices: &mut HashMap<[usize; 2], Vertex>,
    triangles: &mut Vec<Triangle>,
    times: &mut StepTimes,
) -> MeshStats {
    // Locate this tile on the sky and discard projected positions from the previous region.
    times.measure("Projection.Tile geometry");
    let tile = TileGeometry::new(region);
    let projector = TileProjector {
        tile,
        observation,
        camera,
        size,
        half_fov: view.fov_degrees.to_radians() * 0.5,
    };
    vertices.clear();

    // Project the four outer corners as the starting points for the triangle grid.
    times.measure("Projection.Tile corner projection");
    for corner in corner_keys(0, 0, grid_size()) {
        sample_vertex(&projector, vertices, corner);
    }

    // Refine the grid until its sampled error is acceptable or the subdivision limit is reached.
    times.measure("Projection.Adaptive mesh preparation");
    build_mesh(&projector, vertices, triangles)
}

/// Draw all triangles into temporary region pixels, then blend the completed region into the square once.
fn draw_region_mesh(
    stars: &mut [Texel],
    size: usize,
    tile: &crate::model::RegionTileCache,
    triangles: &[Triangle],
    sampler: &mut TileSampler,
    pixels: &mut RegionPixels,
    times: &mut StepTimes,
) {
    // Identify tile areas containing light so empty samples can be skipped.
    times.measure("Projection.Tile filter preparation");
    sampler.prepare(tile.lit_rows());

    // Resize and clear the region's temporary pixels, then fill them through the triangle mapping.
    times.measure("Projection.Tile rasterization");
    pixels.prepare(triangles, size);
    for triangle in triangles {
        pixels.draw_triangle(triangle, sampler, tile.texels());
    }

    // Blend the completed region into the square image.
    times.measure("Projection.Tile composition");
    pixels.composite(stars, size);
}

struct TileProjector<'a> {
    tile: TileGeometry,
    observation: &'a Observation,
    camera: CartesianCamera,
    size: usize,
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

impl MeshStats {
    /// Add this region's counts and retain the largest grid resolution seen in the frame.
    fn add_region(&mut self, region: &Self) {
        self.projections += region.projections;
        self.capped_cells += region.capped_cells;
        self.skipped_cells += region.skipped_cells;
        self.divisions = self.divisions.max(region.divisions);
    }

    /// Finish the projection sub-step timer and describe the combined mesh results.
    fn record_trace(&self, times: &mut StepTimes) {
        times.measure("Projection");
        times.describe("Projection.Adaptive mesh preparation", || format!(
            "{} sky projections; maximum {}x{} cells per region; {} cells above target at limit; {} unsafe cells omitted",
            self.projections, self.divisions, self.divisions, self.capped_cells, self.skipped_cells,
        ));
    }
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
        let radius = (projector.size as f64 - 1.0) * 0.5;
        // The point renderer gives the exact equidistant antipode an arbitrary rim position. A tile vertex
        // must not use that convention: approaching the same direction can reach any point on the rim.
        let antipodal_vertex = projector.camera.kind == ProjectionKind::Equidistant
            && projector.camera.forward.dot(apparent) < 0.0
            && projector.camera.right.dot(apparent).hypot(projector.camera.up.dot(apparent)) < 1e-12;
        let projected = if antipodal_vertex { None } else { project_camera(projector.camera, apparent) };
        let screen = projected.and_then(|point| {
            let screen = [radius + radius * point.x, radius - radius * point.y];
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
/// Invalid cells are refined too; at the limit they are omitted individually. Equidistant also omits cells
/// still above the error target there, avoiding chords across its rear discontinuity.
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
                    // At 360 degrees the equidistant antipode is an entire rim, not one screen point.
                    // Never bridge an unresolved piece with a large chord when the work limit is reached.
                    if divisions == ADAPTIVE_WARP_MAX_SUBDIVISIONS && projector.camera.kind == ProjectionKind::Equidistant {
                        stats.skipped_cells += 1;
                        continue;
                    }
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
    use crate::projection::Frame;
    use super::*;
    use crate::astro::{Observer};
    use crate::constants::GRID_DEPTH;
    use crate::model::{ViewCenter, hash_direction, region_caps};
    use crate::observation::observe;

    fn fixture(fov: f64, atmosphere: bool) -> (View, Observation, CartesianCamera, usize) {
        // Reproduce the screenshot's date, observer, orientation and frame dimensions without loading stars.
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: fov, projection: ProjectionKind::Equidistant };
        let date = crate::astro::parse_utc_datetime("2025-03-01T11:00:00").unwrap();
        let time = crate::astro::utc_jd_to_time_ms(crate::astro::datetime_to_julian_date(&date)).unwrap();
        let site = Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() };
        let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
        (view, observation, prepare_camera(&view), 1116)
    }

    fn projector<'a>(region: usize, observation: &'a Observation, camera: CartesianCamera, size: usize, view: &View) -> TileProjector<'a> {
        // One region's mapping; samples belonging to a different projector must not be reused.
        TileProjector { tile: TileGeometry::new(region), observation, camera, size, half_fov: view.fov_degrees.to_radians() * 0.5 }
    }

    fn exact_position(projector: &TileProjector<'_>, uv: Point) -> Option<Point> {
        // Independent test oracle at arbitrary texture positions, including positions not used by the mesh.
        let catalog = projector.tile.direction_of(uv[0] - 0.5, uv[1] - 0.5);
        let apparent = projector.observation.apparent_star_direction(catalog);
        let point = project_camera(projector.camera, apparent)?;
        let radius = (projector.size as f64 - 1.0) * 0.5;
        Some([radius + point.x * radius, radius - point.y * radius])
    }

    fn is_visible(point: Point, size: usize) -> bool {
        let radius = (size as f64 - 1.0) * 0.5;
        (point[0] - radius).hypot(point[1] - radius) <= radius
    }

    #[test]
    fn flat_regions_stay_coarse_and_vertices_are_reused_only_inside_one_region() {
        // A nearly central tile at a wide view needs few pieces; repeated lookups reuse its exact vertices.
        let (view, observation, camera, size) = fixture(351.6, false);
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let projector = projector(region, &observation, camera, size, &view);
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
    fn mesh_edges_match_and_opaque_pieces_do_not_overlap_or_leave_seams() {
        // A tile near the outer rim requires refinement. Shared grid edges must be exactly equal in screen space.
        let (view, observation, camera, size) = fixture(351.6, false);
        let angle = 175_f64.to_radians();
        let direction = observation.to_catalog.apply(camera.forward * angle.cos() + camera.up * angle.sin());
        let region = hash_direction(GRID_DEPTH, direction);
        let projector = projector(region, &observation, camera, size, &view);
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
        let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));
        let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &Selected { regions: vec![], whole_sky: false }, &observation, &view, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        assert!(frame.stars().iter().all(|texel| texel.is_empty()));

        // Equidistant uses the same adaptive drawing path and still produces finite, bounded pixels.
        let equidistant = View { projection: ProjectionKind::Equidistant, ..view };
        let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &equidistant, &mut StepTimes::default());
        frame.insert_projected_stars(&projected_stars);
        assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
        assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));
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
            crate::regions::refresh_regions(&mut data, &selected, observation.time_ms, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::new(1917, 1116, 1116, (10, 20));
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
                    let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut StepTimes::default());
                    frame.insert_projected_stars(&projected_stars);
                    if run > 0 { elapsed += start.elapsed().as_secs_f64(); }
                    std::hint::black_box(frame.stars());
                }
                eprintln!("Adaptive fov={fov}, dense={dense}, regions={}: {:.3}ms; three frames after warmup", selected.regions.len(), elapsed * 1000.0 / 3.0);
                let mut detail = StepTimes::with_trace(true);
                let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut detail);
                frame.insert_projected_stars(&projected_stars);
                detail.measure("END");
                for step in detail.steps().iter().filter(|step| matches!(step.name, "Adaptive mesh preparation" | "Tile filter preparation" | "Tile rasterization" | "Tile composition")) {
                    eprintln!("  {}: {:.3}ms (separate traced frame)", step.name, step.average_seconds * 1000.0);
                }
            }
        }
    }

    #[test]
    fn equidistant_geometry_covers_normal_and_full_sky_views() {
        // Audit both a normal view and the full-sky rim using the exact equidistant formula as the oracle.
        for (fov, atmosphere) in [(10.0, false), (60.0, true), (180.0, false), (351.6, false), (359.9, false), (360.0, false), (360.0, true)] {
            let (mut view, previous, _, size) = fixture(fov, atmosphere);
            view.projection = ProjectionKind::Equidistant;
            let observation = observe(&Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() }, atmosphere, &view, previous.time_ms, &mut StepTimes::default());
            let camera = prepare_camera(&view);
            let mut samples = HashMap::new();
            let mut mesh = Vec::new();
            let (mut worst, mut probes, mut lost, mut skipped, mut capped) = (0.0_f64, 0, 0, 0, 0);
            for region in 0..region_caps().len() {
                samples.clear();
                let projector = projector(region, &observation, camera, size, &view);
                let stats = build_mesh(&projector, &mut samples, &mut mesh);
                skipped += stats.skipped_cells;
                capped += stats.capped_cells;
                assert!(stats.projections <= (grid_size() + 1).pow(2));
                let visible = [0.1, 0.5, 0.9].into_iter().any(|u| [0.1, 0.5, 0.9].into_iter().any(|v| {
                    exact_position(&projector, [u * TILE_EDGE_TEXELS as f64, v * TILE_EDGE_TEXELS as f64]).is_some_and(|point| is_visible(point, size))
                }));
                if visible && mesh.is_empty() { lost += 1; }
                for triangle in &mesh {
                    let interpolate = |points: [Point; 3]| -> Point { std::array::from_fn(|axis| points.iter().map(|point| point[axis]).sum::<f64>() / 3.0) };
                    let uv = interpolate(triangle.texture);
                    let expected = exact_position(&projector, uv).unwrap();
                    if !is_visible(expected, size) { continue; }
                    let actual = interpolate(triangle.screen);
                    worst = worst.max((expected[0] - actual[0]).hypot(expected[1] - actual[1]));
                    probes += 1;
                }
            }
            eprintln!("Equidistant fov={fov} atmosphere={atmosphere}: probes={probes}, worst={worst:.6}px, lost_regions={lost}, skipped_cells={skipped}, capped_cells={capped}");
            assert!(probes > 0);
            assert_eq!(lost, 0);
            assert!(worst < 1.0, "equidistant interpolation error {worst}px at {fov} degrees");
        }
    }

    #[test]
    fn an_exact_equidistant_antipode_does_not_anchor_a_tile_to_one_rim_point() {
        // Aim the camera so one tile corner lies exactly behind it. Point markers keep their existing
        // arbitrary-pole convention, but a surface vertex must be treated as singular.
        let (_, observation, _, _) = fixture(360.0, false);
        let region = hash_direction(GRID_DEPTH, observation.patch.center);
        let tile = TileGeometry::new(region);
        let apparent = observation.apparent_star_direction(tile.direction_of(-0.5, -0.5));
        let facing = crate::astro::Horizontal::from_vector(-apparent);
        let view = View { center: ViewCenter::Facing { azimuth: facing.azimuth, tilt: facing.altitude },
            projection: ProjectionKind::Equidistant, fov_degrees: 360.0 };
        let camera = prepare_camera(&view);
        let size = 201;
        let projector = projector(region, &observation, camera, size, &view);
        assert!(project_camera(camera, apparent).is_some());
        assert!(sample_vertex(&projector, &mut HashMap::new(), [0, 0]).screen.is_none());

        // A fully illuminated rear tile must stay at the outer rim, never draw a chord through the center.
        let mut data = Data::default();
        let image = &mut data.star_regions[region].merged_tile;
        image.tile.texels = vec![Texel { rgb: [1.0; 3], opacity: 1.0 }; TILE_EDGE_TEXELS.pow(2)];
        image.lit_rows = vec![u64::MAX; TILE_EDGE_TEXELS];
        let selected = Selected { regions: vec![region], whole_sky: true };
        let mut frame = Frame::square(201);
        let mut times = StepTimes::with_trace(true);
        let projected_stars = project_adaptive_tiles(frame.drawing_area_width_and_height, &data, &selected, &observation, &view, &mut times);
        frame.insert_projected_stars(&projected_stars);
        times.measure("END");
        assert!(times.steps().iter().any(|step| step.name == "Adaptive mesh preparation"));
        assert!(frame.stars().iter().any(|texel| texel.opacity > 0.0));
        for (index, texel) in frame.stars().iter().enumerate() {
            assert!(texel.opacity.is_finite() && texel.opacity <= 1.0);
            if texel.opacity > 0.0 {
                let radius = (index % 201) as f64 - 100.0;
                let vertical = (index / 201) as f64 - 100.0;
                assert!(radius.hypot(vertical) > 90.0, "rear tile stretched into the disc at {index}");
            }
        }
    }

}
