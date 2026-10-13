//! Four-corner forward mapping of each merged region tile. Only the outer corners go through the astronomy
//! and camera formulas. Two textured triangles interpolate the interior, so curved edges are approximated.
//! A temporary box filter averages texels during reduction; it approximates rotated pixel footprints by their
//! texture-space bounding boxes. Region order and premultiplied source-over composition remain unchanged.
use crate::astro::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{Data, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, prepare_camera, project_camera};
use crate::regions::TileGeometry;
use crate::selection::Selected;
use crate::timing::StepTimes;
use super::raster::{Point, TileRaster};
use super::tile_sampling::TileSampler;

pub(super) fn project_whole_tiles(frame: &mut Frame, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes) {
    // Clear the previous sky and prepare the current camera and screen dimensions.
    times.measure_detail("Star layer clear", || frame.stars_mut().fill(Texel::default()));
    let (width, height, sky) = (frame.width(), frame.height(), frame.sky());
    if width == 0 || height == 0 || sky.size < 2 { return; }
    let camera = times.measure_detail("Camera preparation", || prepare_camera(view));

    // Reuse these temporary buffers between regions; no projected result is cached between frames.
    let mut raster = times.measure_detail("Raster scratch allocation", || TileRaster::new(width, height, sky));
    let mut sampler = times.measure_detail("Tile filter allocation", || TileSampler::new(TILE_EDGE_TEXELS));
    let mut passes = times.detail_batch([
        "Tile geometry",
        "Tile corner projection",
        "Tile filter preparation",
        "Triangle rasterization",
        "Tile composition",
    ]);

    // Project each tile once, sample its two triangles, then blend the complete region onto the frame.
    for &region in &selected.regions {
        let image = &data.star_regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        let geometry = passes.measure(0, || TileGeometry::new(region));
        let corners = passes.measure(1, || project_tile_corners(&geometry, observation, camera, sky));
        let Some(corners) = corners else { continue; };

        passes.measure(2, || sampler.prepare(image.texels()));
        passes.measure(3, || draw_tile(&mut raster, corners, &sampler));
        passes.measure(4, || raster.composite(frame.stars_mut()));
    }
    passes.publish(times);

    // Include buffer release in the timing breakdown.
    times.measure_detail("Raster scratch release", || drop(raster));
    times.measure_detail("Tile filter release", || drop(sampler));
}

/// Project the four outer texture boundaries. Skip tiles crossing the back singularity or folding over
/// themselves: a two-triangle surface cannot represent those mappings reliably.
fn project_tile_corners(tile: &TileGeometry, observation: &Observation, camera: CartesianCamera, sky: SkyArea) -> Option<[Point; 4]> {
    // Texture boundaries extend half a texel beyond the first and last texel centers.
    let minimum = -0.5;
    let maximum = TILE_EDGE_TEXELS as f64 - 0.5;
    let tile_corners = [(minimum, minimum), (maximum, minimum), (maximum, maximum), (minimum, maximum)];
    let directions = tile_corners.map(|(x, y)| {
        observation.apparent_star_direction(tile.direction_of(x, y))
    });
    if contains_antipode(directions, camera.forward) { return None; }

    // Run the real camera projection only at these four corners.
    let radius = (sky.size as f64 - 1.0) * 0.5;
    let mut corners = [[0.0; 2]; 4];
    for (corner, direction) in corners.iter_mut().zip(directions) {
        let projected = project_camera(camera, direction)?;
        if !projected.x.is_finite() || !projected.y.is_finite() { return None; }
        *corner = [sky.x as f64 + radius + projected.x * radius, sky.y as f64 + radius - projected.y * radius];
    }

    // Accept either winding, but reject collapsed or nonconvex quadrilaterals in this prototype.
    if !is_convex(corners) { return None; }
    Some(corners)
}

/// Test the small spherical quad against the direction directly behind the camera. Boundary points count as
/// inside: the projection is singular there too. Texels away from the back hemisphere take the quick exit.
fn contains_antipode(corners: [Vector3; 4], forward: Vector3) -> bool {
    if corners.iter().any(|corner| corner.dot(forward) >= 0.0) { return false; }
    let centre = corners.into_iter().fold(Vector3::default(), |sum, corner| sum + corner);
    (0..4).all(|i| {
        let normal = corners[i].cross(corners[(i + 1) % 4]);
        normal.dot(forward * -1.0) * normal.dot(centre) >= 0.0
    })
}

fn is_convex(corners: [Point; 4]) -> bool {
    // Each consecutive pair of edges must turn in the same direction.
    let turns: [f64; 4] = std::array::from_fn(|index| {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let c = corners[(index + 2) % 4];
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    });
    turns.iter().all(|turn| turn.is_finite())
        && (turns.iter().all(|&turn| turn > 0.0) || turns.iter().all(|&turn| turn < 0.0))
}

fn draw_tile(raster: &mut TileRaster, corners: [Point; 4], sampler: &TileSampler) {
    // Match the screen corners to the boundaries of the full texture.
    let edge = TILE_EDGE_TEXELS as f64;
    let texture = [[0.0, 0.0], [edge, 0.0], [edge, edge], [0.0, edge]];

    // Accumulate both halves into the same tile layer, avoiding a double blend along the diagonal.
    for indices in [[0, 1, 2], [0, 2, 3]] {
        let screen_triangle = indices.map(|index| corners[index]);
        let texture_triangle = indices.map(|index| texture[index]);
        let Some(mapping) = TextureMapping::new(screen_triangle, texture_triangle) else { continue; };
        raster.draw_textured_triangle(screen_triangle, |pixel| {
            sampler.sample(mapping.texture_position(pixel), mapping.footprint)
        });
    }
}

/// Affine screen-to-texture mapping for one triangle. Derivatives are calculated once per triangle, not pixel.
struct TextureMapping {
    screen_origin: Point,
    texture_origin: Point,
    per_screen_x: Point,
    per_screen_y: Point,
    footprint: Point,
}

impl TextureMapping {
    fn new(screen: [Point; 3], texture: [Point; 3]) -> Option<Self> {
        // Invert the triangle's two screen-space edges; a collapsed triangle has no usable mapping.
        let screen_first = [screen[1][0] - screen[0][0], screen[1][1] - screen[0][1]];
        let screen_second = [screen[2][0] - screen[0][0], screen[2][1] - screen[0][1]];
        let determinant = screen_first[0] * screen_second[1] - screen_second[0] * screen_first[1];
        if determinant == 0.0 || !determinant.is_finite() { return None; }

        // Calculate how one screen-pixel step changes the two texture coordinates.
        let mut per_screen_x = [0.0; 2];
        let mut per_screen_y = [0.0; 2];
        for axis in 0..2 {
            let texture_first = texture[1][axis] - texture[0][axis];
            let texture_second = texture[2][axis] - texture[0][axis];
            per_screen_x[axis] = (texture_first * screen_second[1] - texture_second * screen_first[1]) / determinant;
            per_screen_y[axis] = (texture_second * screen_first[0] - texture_first * screen_second[0]) / determinant;
        }
        let footprint = std::array::from_fn(|axis| per_screen_x[axis].abs() + per_screen_y[axis].abs());
        if footprint.iter().any(|value: &f64| !value.is_finite()) { return None; }

        Some(Self { screen_origin: screen[0], texture_origin: texture[0], per_screen_x, per_screen_y, footprint })
    }

    fn texture_position(&self, pixel: Point) -> Point {
        // Interpolate inside this triangle instead of repeating sky and camera calculations per texel.
        let delta_x = pixel[0] - self.screen_origin[0];
        let delta_y = pixel[1] - self.screen_origin[1];
        std::array::from_fn(|axis| {
            self.texture_origin[axis] + delta_x * self.per_screen_x[axis] + delta_y * self.per_screen_y[axis]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, J2000, Observer};
    use crate::model::{ProjectionKind, RenderOptions, ViewCenter, fixtures, hash_direction};
    use crate::observation::observe;
    use crate::regions::refresh_regions;

    #[test]
    fn antipodal_tiles_are_rejected_without_rejecting_nearby_back_facing_tiles() {
        let forward = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let directions = |offset| [(-0.01 + offset, -0.01), (0.01 + offset, -0.01), (0.01 + offset, 0.01), (-0.01 + offset, 0.01)]
            .map(|(x, y)| Vector3 { x, y, z: -1.0 }.normalized());
        assert!(contains_antipode(directions(0.0), forward));
        assert!(!contains_antipode(directions(0.1), forward));
    }

    #[test]
    fn mapping_handles_skew_and_reversed_winding() {
        let screen = [[10.0, 20.0], [18.0, 22.0], [12.0, 26.0]];
        let texture = [[0.0, 0.0], [64.0, 0.0], [0.0, 64.0]];
        for indices in [[0, 1, 2], [0, 2, 1]] {
            let mapping = TextureMapping::new(indices.map(|i| screen[i]), indices.map(|i| texture[i])).unwrap();
            for (point, expected) in screen.into_iter().zip(texture) {
                for (actual, expected) in mapping.texture_position(point).into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
        assert!(TextureMapping::new([[0.0; 2]; 3], texture).is_none());
        assert!(!is_convex([[0.0; 2]; 4]));
        assert!(!is_convex([[0.0, 0.0], [1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]));
    }

    #[test]
    fn a_whole_tile_reduced_to_one_pixel_keeps_off_center_star_light() {
        let edge = TILE_EDGE_TEXELS;
        let mut texels = vec![Texel::default(); edge * edge];
        texels[0] = Texel { rgb: [0.8, 0.4, 0.2], opacity: 0.8 };
        let mut sampler = TileSampler::new(edge);
        sampler.prepare(&texels);
        let mut raster = TileRaster::new(9, 9, SkyArea { x: 0, y: 0, size: 9 });
        let mut frame = vec![Texel::default(); 81];
        draw_tile(&mut raster, [[3.5, 3.5], [4.5, 3.5], [4.5, 4.5], [3.5, 4.5]], &sampler);
        raster.composite(&mut frame);
        let expected = texels[0].scaled(1.0 / (edge * edge) as f32);
        assert_eq!(frame[40], expected);
        assert_eq!(frame.iter().filter(|texel| !texel.is_empty()).count(), 1);
    }

    #[test]
    fn two_triangles_cover_once_and_region_layers_keep_source_over_order() {
        let edge = TILE_EDGE_TEXELS;
        let mut sampler = TileSampler::new(edge);
        let mut raster = TileRaster::new(9, 9, SkyArea { x: 0, y: 0, size: 9 });
        let mut frame = vec![Texel::default(); 81];
        let corners = [[2.5, 2.5], [5.5, 2.5], [5.5, 5.5], [2.5, 5.5]];
        for color in [[0.5, 0.0, 0.0], [0.0, 0.0, 0.5]] {
            sampler.prepare(&vec![Texel { rgb: color, opacity: 0.5 }; edge * edge]);
            draw_tile(&mut raster, corners, &sampler);
            raster.composite(&mut frame);
        }
        assert_eq!(frame[40], Texel { rgb: [0.25, 0.0, 0.5], opacity: 0.75 });
        assert!(frame[0].is_empty());
    }

    #[test]
    fn whole_tiles_place_stars_near_their_observed_positions_and_clear_empty_views() {
        let time = FrameTime::from_utc(J2000);
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        for (fov, atmosphere, projection) in [(180.0, false, ProjectionKind::Stereographic), (60.0, true, ProjectionKind::Stereographic),
            (10.0, true, ProjectionKind::Equidistant)] {
            let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.02 }, projection, fov_degrees: fov };
            let observation = observe(&site, atmosphere, &view, time, &mut StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(crate::constants::GRID_DEPTH, direction);
            let star = fixtures::row(direction, 0.0, 3.0, 1);
            let mut data = fixtures::layout(&[(region, 0, star)]);
            let selected = Selected { regions: vec![region], whole_sky: false };
            let settings = RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
            refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
            let mut frame = Frame::square(401);
            project_whole_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());

            let expected = project_camera(prepare_camera(&view), observation.apparent_star_direction(direction)).unwrap();
            let expected = [200.0 + expected.x * 200.0, 200.0 - expected.y * 200.0];
            let mut total = 0.0;
            let mut center = [0.0; 2];
            for (index, texel) in frame.stars().iter().enumerate() {
                let weight = texel.opacity as f64;
                total += weight;
                center[0] += (index % 401) as f64 * weight;
                center[1] += (index / 401) as f64 * weight;
            }
            assert!(total > 0.0);
            assert!((center[0] / total - expected[0]).abs() < 3.0 && (center[1] / total - expected[1]).abs() < 3.0,
                "{fov}: center {center:?} / {total}, expected {expected:?}");
            assert!(frame.stars().iter().all(|texel| texel.opacity.is_finite() && texel.opacity <= 1.0));

            project_whole_tiles(&mut frame, &data, &Selected { regions: Vec::new(), whole_sky: false }, &observation, &view, &mut StepTimes::default());
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }

    #[test]
    #[ignore = "timing only; measures whole-tile mapping without loading external datasets"]
    fn time_whole_tile_mapping() {
        // Prepare a real small catalog once; the dense variant fills the same selected tiles synthetically.
        let mut report = crate::progress::Report::quiet();
        let prepared = crate::sky::prepare_owned_catalog(crate::catalog::load_embedded_catalog(&mut report).unwrap(), &mut report).unwrap();
        let mut data = crate::sky::build_sky_tables(&prepared, &mut report).unwrap();
        let time = FrameTime::from_utc(J2000);
        let view = View { center: ViewCenter::Facing { azimuth: 334_f64.to_radians(), tilt: 0.0 }, fov_degrees: 180.0, ..View::default() };
        let observation = observe(&Observer { latitude: 35.69_f64.to_radians(), longitude: 139.69_f64.to_radians() }, true, &view, time, &mut StepTimes::default());
        let selected = crate::selection::select_regions(&data.star_regions, &observation, &mut StepTimes::default());
        let settings = RenderOptions { magnitude_threshold: 9.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false };
        refresh_regions(&mut data, &selected, time, &view, &settings, &mut StepTimes::default());
        let mut frame = Frame::new(1920, 1100, SkyArea::centred(1920, 1100), (10, 20));

        for dense in [false, true] {
            if dense {
                for &region in &selected.regions {
                    let image = &mut data.star_regions[region].merged_tile;
                    image.tile.texels = vec![Texel { rgb: [0.1, 0.15, 0.2], opacity: 0.3 }; TILE_EDGE_TEXELS * TILE_EDGE_TEXELS];
                    image.lit_rows = vec![u64::MAX >> (64 - TILE_EDGE_TEXELS); TILE_EDGE_TEXELS];
                }
            }

            // Exclude the first frame, then average five whole-tile renders without detailed timers.
            let mut elapsed = 0.0;
            for run in 0..6 {
                let start = std::time::Instant::now();
                project_whole_tiles(&mut frame, &data, &selected, &observation, &view, &mut StepTimes::default());
                let seconds = start.elapsed().as_secs_f64();
                if run > 0 { elapsed += seconds; }
                std::hint::black_box(frame.stars());
            }
            eprintln!("dense={dense}; selected={}; whole_tile_ms={:.3}; five frames, timing detail disabled",
                selected.regions.len(), elapsed * 200.0);
        }
    }
}
