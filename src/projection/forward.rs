//! Forward tile-mapping prototype. Each selected tile is drawn independently, including its padded edges.
//! Every lit texel's four corners go through the real observation/projection formulas; two triangles approximate
//! the curved mapping over that one texel. No screen-to-sky rays or cross-region corner sharing are used.
//!
//! Texels contribute premultiplied RGBA in proportion to covered screen-pixel area (ordinary image reduction,
//! not a sum of texel energy). Pieces within a tile accumulate before the tile blends source-over onto the frame.
//! Region order is selection order. The original inverse warp remains available through the constant switch.
use crate::timing::StepTimes;
use crate::astro::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::View;
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, prepare_camera, project_camera};
use crate::regions::{RegionCaches, Texel, TileGeometry, blend_star};
use crate::selection::Selected;

type Point = [f64; 2];

/// Draw all nonempty texels of each selected tile; scratch storage is reused between tiles, never cached.
pub(super) fn project_tiles_forward(frame: &mut Frame, caches: &RegionCaches, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes) {
    times.measure_detail("Star layer clear", || frame.stars_mut().fill(Texel::default()));
    let (width, height, sky) = (frame.width(), frame.height(), frame.sky());
    if width == 0 || height == 0 || sky.size < 2 { return; }
    let camera = times.measure_detail("Camera preparation", || prepare_camera(view));
    let mut raster = times.measure_detail("Raster scratch allocation", || TileRaster { pixels: vec![[0.0; 4]; width * height], touched: Vec::new(), width, height, sky });
    let mut passes = times.detail_batch(["Tile geometry", "Texel projection", "Triangle rasterization", "Tile composition"]);
    for &region in &selected.regions {
        let image = caches.region_image(region);
        if image.is_empty() { continue; }
        let geometry = passes.measure(0, || TileGeometry::new(region));
        for (row, &lit) in image.lit_rows().iter().enumerate() {
            let mut bits = lit;
            while bits != 0 {
                let column = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let texel = image.texels()[row * TILE_EDGE_TEXELS + column];
                let Some(corners) = passes.measure(1, || project_texel(&geometry, column, row, observation, camera, sky)) else { continue; };
                passes.measure(2, || {
                    raster.draw_triangle([corners[0], corners[1], corners[2]], texel);
                    raster.draw_triangle([corners[0], corners[2], corners[3]], texel);
                });
            }
        }
        passes.measure(3, || raster.composite(frame.stars_mut()));
    }
    passes.publish(times);
    times.measure_detail("Raster scratch release", || drop(raster));
}

/// Evaluate the full nonlinear mapping at a texel's corners, rather than interpolating from a whole tile's
/// four outer corners. A texel crossing the antipodal projection singularity is skipped in this prototype.
fn project_texel(tile: &TileGeometry, column: usize, row: usize, observation: &Observation, camera: CartesianCamera, sky: SkyArea) -> Option<[Point; 4]> {
    let (left, top) = (column as f64 - 0.5, row as f64 - 0.5);
    let directions = [(left, top), (left + 1.0, top), (left + 1.0, top + 1.0), (left, top + 1.0)]
        .map(|(x, y)| observation.apparent_star_direction(tile.direction_of(x, y)));
    if contains_antipode(directions, camera.forward) { return None; }                     // never stretch a texel across the whole screen through the back of the projection
    let radius = (sky.size as f64 - 1.0) * 0.5;
    let mut corners = [[0.0; 2]; 4];
    for (corner, direction) in corners.iter_mut().zip(directions) {
        let projected = project_camera(camera, direction)?;
        if !projected.x.is_finite() || !projected.y.is_finite() { return None; }
        *corner = [sky.x as f64 + radius + projected.x * radius, sky.y as f64 + radius - projected.y * radius];
    }
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

/// Per-frame scratch: accumulate a tile's coverage in f64, then touch only its painted pixels when composing
/// and clearing it. Adjacent texels and the two halves of one texel must not blend as separate opaque layers.
struct TileRaster {
    pixels: Vec<[f64; 4]>,
    touched: Vec<usize>,
    width: usize,
    height: usize,
    sky: SkyArea,
}

impl TileRaster {
    fn draw_triangle(&mut self, triangle: [Point; 3], texel: Texel) {
        if texel.is_empty() { return; }
        let minimum = [0, 1].map(|axis| triangle.iter().map(|point| point[axis]).fold(f64::INFINITY, f64::min));
        let maximum = [0, 1].map(|axis| triangle.iter().map(|point| point[axis]).fold(f64::NEG_INFINITY, f64::max));
        let (left, right) = ((minimum[0] - 0.5).ceil().max(0.0), (maximum[0] + 0.5).floor().min((self.width - 1) as f64));
        let (top, bottom) = ((minimum[1] - 0.5).ceil().max(0.0), (maximum[1] + 0.5).floor().min((self.height - 1) as f64));
        if left > right || top > bottom { return; }
        let radius = (self.sky.size as f64 - 1.0) * 0.5;
        let (cx, cy) = (self.sky.x as f64 + radius, self.sky.y as f64 + radius);
        for row in top as usize..=bottom as usize {
            for column in left as usize..=right as usize {
                if (column as f64 - cx).powi(2) + (row as f64 - cy).powi(2) > radius * radius + 1e-9 { continue; }
                let coverage = triangle_coverage(triangle, [column as f64, row as f64]);
                if coverage == 0.0 { continue; }
                let index = row * self.width + column;
                if self.pixels[index][3] == 0.0 { self.touched.push(index); }
                for (sum, value) in self.pixels[index].iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) {
                    *sum += value as f64 * coverage;
                }
            }
        }
    }

    fn composite(&mut self, frame: &mut [Texel]) {
        for index in self.touched.drain(..) {
            let rgba = self.pixels[index];
            #[cfg(feature = "pixel-debug")]
            crate::pixel_debug::check_pixel("forward tile coverage before clamp", rgba, 1.0);
            let scale = if rgba[3] > 1.0 { 1.0 / rgba[3] } else { 1.0 };                   // absorb final-bit coverage noise at opaque edges
            let rgb = [rgba[0], rgba[1], rgba[2]].map(|value| (value * scale) as f32);
            blend_star(&mut frame[index], rgb, rgba[3].min(1.0) as f32);
            self.pixels[index] = [0.0; 4];
        }
    }
}

/// Area of a projected triangle inside one unit screen pixel. Clipping uses fixed stack buffers; the intersection
/// of a triangle and a rectangle has at most seven vertices. Pixel-relative coordinates avoid cancellation.
fn triangle_coverage(triangle: [Point; 3], pixel: Point) -> f64 {
    let mut vertices = [[0.0; 2]; 8];
    for (vertex, point) in vertices.iter_mut().zip(triangle) { *vertex = [point[0] - pixel[0], point[1] - pixel[1]]; }
    let mut count = 3;
    for axis in [0, 1] {
        for sign in [-1.0, 1.0] {
            let input = vertices;
            let mut previous = input[count - 1];
            let mut previous_distance = 0.5 - sign * previous[axis];
            let mut written = 0;
            for point in input[..count].iter().copied() {
                let distance = 0.5 - sign * point[axis];
                if (previous_distance >= 0.0) != (distance >= 0.0) {
                    let fraction = previous_distance / (previous_distance - distance);
                    vertices[written] = [previous[0] + fraction * (point[0] - previous[0]), previous[1] + fraction * (point[1] - previous[1])];
                    written += 1;
                }
                if distance >= 0.0 { vertices[written] = point; written += 1; }
                previous = point;
                previous_distance = distance;
            }
            count = written;
            if count < 3 { return 0.0; }
        }
    }
    let mut twice_area = 0.0;
    let mut previous = vertices[count - 1];
    for point in vertices[..count].iter().copied() {
        twice_area += previous[0] * point[1] - previous[1] * point[0];
        previous = point;
    }
    let area = twice_area.abs() * 0.5;
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_channel("forward triangle coverage before clamp", 3, area, 1.0);
    area.min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{FrameTime, J2000, Observer};
    use crate::model::{ProjectionKind, RenderOptions, SkyTables, ViewCenter, fixtures, hash_direction};
    use crate::observation::observe;
    use crate::regions::refresh_regions;

    fn raster() -> TileRaster {
        TileRaster { pixels: vec![[0.0; 4]; 9 * 9], touched: Vec::new(), width: 9, height: 9, sky: SkyArea { x: 0, y: 0, size: 9 } }
    }
    fn square(raster: &mut TileRaster, low: f64, high: f64, texel: Texel) {
        let corners = [[low, low], [high, low], [high, high], [low, high]];
        raster.draw_triangle([corners[0], corners[1], corners[2]], texel);
        raster.draw_triangle([corners[0], corners[2], corners[3]], texel);
    }
    fn close(actual: Texel, expected: Texel) {
        for (a, b) in actual.rgb.into_iter().chain([actual.opacity]).zip(expected.rgb.into_iter().chain([expected.opacity])) {
            assert!((a - b).abs() < 1e-6, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn triangle_halves_cover_one_pixel_once_and_reduction_uses_screen_area() {
        let light = Texel { rgb: [0.8, 0.4, 0.2], opacity: 0.8 };
        let mut raster = raster();
        let mut frame = vec![Texel::default(); 81];
        square(&mut raster, 3.5, 4.5, light);
        raster.composite(&mut frame);
        close(frame[40], light);                                                         // two half-pixel triangles, not two alpha blends
        assert_eq!(frame.iter().filter(|texel| !texel.is_empty()).count(), 1);
        assert!(raster.touched.is_empty() && raster.pixels.iter().all(|pixel| *pixel == [0.0; 4]));

        frame.fill(Texel::default());
        square(&mut raster, 3.75, 4.25, light);                                            // a half-width texel covers a quarter screen pixel
        raster.composite(&mut frame);
        close(frame[40], light.scaled(0.25));
    }

    #[test]
    fn adjacent_texels_accumulate_coverage_but_overlapping_tiles_blend_source_over() {
        let mut raster = raster();
        let mut frame = vec![Texel::default(); 81];
        let red = Texel { rgb: [0.5, 0.0, 0.0], opacity: 0.5 };
        let blue = Texel { rgb: [0.0, 0.0, 0.5], opacity: 0.5 };
        square(&mut raster, 3.5, 4.5, red);
        raster.composite(&mut frame);
        square(&mut raster, 3.5, 4.5, blue);
        raster.composite(&mut frame);
        close(frame[40], Texel { rgb: [0.25, 0.0, 0.5], opacity: 0.75 });

        frame.fill(Texel::default());
        square(&mut raster, 3.0, 4.0, red);                                               // four adjacent texels meeting inside the central pixel
        square(&mut raster, 4.0, 5.0, red);
        for triangle in [[[4.0, 3.0], [5.0, 3.0], [5.0, 4.0]], [[4.0, 3.0], [5.0, 4.0], [4.0, 4.0]],
            [[3.0, 4.0], [4.0, 4.0], [4.0, 5.0]], [[3.0, 4.0], [4.0, 5.0], [3.0, 5.0]]] {
            raster.draw_triangle(triangle, red);
        }
        raster.composite(&mut frame);
        close(frame[40], red);                                                           // four quarters average to the same colour and opacity
    }

    #[test]
    fn coverage_handles_reversed_triangles_partial_edges_and_clipping() {
        let triangle = [[3.5, 3.5], [4.5, 3.5], [3.5, 4.5]];
        assert!((triangle_coverage(triangle, [4.0, 4.0]) - 0.5).abs() < 1e-12);
        assert_eq!(triangle_coverage(triangle, [4.0, 4.0]), triangle_coverage([triangle[2], triangle[1], triangle[0]], [4.0, 4.0]));
        assert_eq!(triangle_coverage([[4.0, 4.0]; 3], [4.0, 4.0]), 0.0);
        let mut raster = raster();
        let light = Texel { rgb: [1.0; 3], opacity: 1.0 };
        square(&mut raster, -100.0, 100.0, light);                                         // vertices outside the frame can enclose visible pixels
        let mut frame = vec![Texel::default(); 81];
        raster.composite(&mut frame);
        close(frame[40], light);
        assert!(frame[0].is_empty() && frame[80].is_empty());                              // circular sky mask, not the enclosing square
    }

    #[test]
    fn antipodal_texels_are_rejected_without_rejecting_nearby_back_facing_texels() {
        let forward = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
        let directions = |offset| [(-0.01 + offset, -0.01), (0.01 + offset, -0.01), (0.01 + offset, 0.01), (-0.01 + offset, 0.01)]
            .map(|(x, y)| Vector3 { x, y, z: -1.0 }.normalized());
        assert!(contains_antipode(directions(0.0), forward));
        assert!(!contains_antipode(directions(0.1), forward));
    }

    #[test]
    fn a_forward_star_lands_at_its_observed_position_and_empty_selection_clears_the_frame() {
        let time = FrameTime::from_utc(J2000);
        let site = Observer { latitude: 0.6, longitude: 2.4 };
        for (fov, atmosphere, projection) in [(180.0, false, ProjectionKind::Stereographic), (60.0, true, ProjectionKind::Stereographic),
            (10.0, true, ProjectionKind::Equidistant)] {
            let view = View { center: ViewCenter::Facing { azimuth: 1.0, tilt: 0.02 }, projection, fov_degrees: fov };
            let observation = observe(&site, atmosphere, &view, time, &mut crate::timing::StepTimes::default());
            let direction = observation.patch.center;
            let region = hash_direction(crate::constants::GRID_DEPTH, direction);
            let row = fixtures::row(direction, 0.0, 3.0, 1);
            let tables = SkyTables { stars: fixtures::layout(&[(region, 0, row)]), ..SkyTables::default() };
            let selected = Selected { regions: vec![region], whole_sky: false };
            let mut caches = RegionCaches::default();
            refresh_regions(&mut caches, &tables, &selected, time, &view, &RenderOptions { magnitude_threshold: 6.0, unicode: true, braille: false, color: true, constellations: false, grid: false, dynamic_names: false }, &mut crate::timing::StepTimes::default());
            let mut frame = Frame::square(401);
            project_tiles_forward(&mut frame, &caches, &selected, &observation, &view, &mut crate::timing::StepTimes::default());
            let expected = project_camera(prepare_camera(&view), observation.apparent_star_direction(direction)).unwrap();
            let expected = [200.0 + expected.x * 200.0, 200.0 - expected.y * 200.0];
            let mut total = 0.0;
            let mut centre = [0.0; 2];
            for (index, texel) in frame.stars().iter().enumerate() {
                let weight = texel.opacity as f64;
                total += weight;
                centre[0] += (index % 401) as f64 * weight;
                centre[1] += (index / 401) as f64 * weight;
            }
            assert!(total > 0.0);
            assert!((centre[0] / total - expected[0]).abs() < 3.0 && (centre[1] / total - expected[1]).abs() < 3.0, "{fov}: {centre:?} / {total}, expected {expected:?}");
            assert!(frame.stars().iter().all(|texel| texel.opacity <= 1.0 && texel.opacity.is_finite()));
            assert!(frame.rays().key().is_none());                                         // forward mapping did not prepare screen-to-sky rays
            project_tiles_forward(&mut frame, &caches, &Selected { regions: Vec::new(), whole_sky: false }, &observation, &view, &mut crate::timing::StepTimes::default());
            assert!(frame.stars().iter().all(|texel| texel.is_empty()));
        }
    }
}
