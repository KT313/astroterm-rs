//! Shared bounded tessellation and coverage for replaceable forward curved-patch backends.
use crate::constants::{CURVED_PATCH_FLATNESS_PIXELS, CURVED_PATCH_MAX_SUBDIVISIONS, TILE_EDGE_TEXELS};
use crate::model::{Data, ProjectionKind, Texel, View};
use crate::observation::Observation;
use crate::projection::{CartesianCamera, Frame, SkyArea, prepare_camera};
use crate::regions::{TileGeometry, blend_star};
use crate::selection::Selected;
use crate::time::StepTimes;
use super::raster::Point;
use super::tile_sampling::TileSampler;
use super::tile_warp::{TextureMapping, is_convex};

/// A fitted mapping from tile coordinates (0..1) to screen coordinates. Evaluating it never projects sky rays.
pub(super) trait CurvedPatch {
    fn evaluate(&self, u: f64, v: f64) -> Point;
}

/// Forward-render stereographic tiles as curved patches; equidistant keeps the existing straight renderer.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_curved_tiles<P: CurvedPatch>(
    frame: &mut Frame, data: &Data, selected: &Selected, observation: &Observation, view: &View, times: &mut StepTimes,
    build_patch: fn(&TileGeometry, &Observation, CartesianCamera, SkyArea) -> Option<P>, mesh_stage: &'static str,
) {
    // Limit this prototype to stereographic views without changing the existing -e behavior.
    if view.projection != ProjectionKind::Stereographic {
        super::tile_warp::project_whole_tiles(frame, data, selected, observation, view, times);
        return;
    }

    // Clear the last frame and prepare the camera once for every selected region.
    times.measure("Projection.Star layer clear");
    frame.stars_mut().fill(Texel::default());
    let (width, height, sky) = (frame.width(), frame.height(), frame.sky());
    if width == 0 || height == 0 || sky.size < 2 { return; }
    times.measure("Projection.Camera preparation");
    let camera = prepare_camera(view);
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS);
    let mut mesh = Vec::new();
    let mut pixels = RegionPixels::default();

    // Fit one patch per region, then reuse temporary mesh and pixel storage for the next region.
    for &region in &selected.regions {
        let image = &data.star_regions[region].merged_tile;
        if image.is_empty() || image.lit_rows().iter().all(|&row| row == 0) { continue; }
        times.measure("Projection.Tile geometry");
        let geometry = TileGeometry::new(region);
        times.measure("Projection.Tile corner projection");
        let patch = build_patch(&geometry, observation, camera, sky);
        let Some(patch) = patch else { continue; };
        times.measure(mesh_stage);
        if !build_mesh(&patch, &mut mesh) { continue; }
        times.measure("Projection.Tile filter preparation");
        sampler.prepare(image.lit_rows());
        times.measure("Projection.Tile rasterization");
        pixels.prepare(&mesh, width, height, sky);
        for triangle in &mesh {
            pixels.draw_triangle(triangle, &sampler, image.texels());
        }
        times.measure("Projection.Tile composition");
        pixels.composite(frame.stars_mut(), width);
    }
    times.measure("Projection");
}

pub(super) struct Triangle {
    pub(super) screen: [Point; 3],
    mapping: TextureMapping,
    #[cfg(test)]
    pub(super) texture: [Point; 3],          // inspect interpolation error in tests without altering rendering
}

impl Triangle {
    pub(super) fn new(screen: [Point; 3], texture: [Point; 3]) -> Option<Self> {
        // Each piece reads its own part of the original tile through one affine texture mapping.
        let mapping = TextureMapping::new(screen, texture)?;
        Some(Self { screen, mapping, #[cfg(test)] texture })
    }
}

/// Subdivide the polynomial patch until sampled flatness is sufficient or the bounded work limit is reached.
/// This controls rasterization of the fitted patch; it is not an error guarantee against the true projection.
pub(super) fn build_mesh(patch: &impl CurvedPatch, mesh: &mut Vec<Triangle>) -> bool {
    // Double a uniform grid so adjacent cells always share the same edge vertices (no T-junctions).
    let mut divisions = 1;
    loop {
        mesh.clear();
        let mut maximum_error: f64 = 0.0;
        let mut winding: f64 = 0.0;
        for row in 0..divisions {
            for column in 0..divisions {
                let (u0, u1) = (column as f64 / divisions as f64, (column + 1) as f64 / divisions as f64);
                let (v0, v1) = (row as f64 / divisions as f64, (row + 1) as f64 / divisions as f64);
                let coordinates = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                let corners = coordinates.map(|[u, v]| patch.evaluate(u, v));
                if !is_convex(corners) { mesh.clear(); return false; }
                let turn = cross(corners[0], corners[1], corners[2]);
                if winding != 0.0 && winding.signum() != turn.signum() { mesh.clear(); return false; }
                winding = turn;

                // Check quarter points as well as midpoints: an S-shaped edge can have a straight midpoint.
                for (start, end) in [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)] {
                    for fraction in [0.25, 0.5, 0.75] {
                        let interpolate = |first: Point, second: Point| -> Point {
                            std::array::from_fn(|axis| (1.0 - fraction) * first[axis] + fraction * second[axis])
                        };
                        let [u, v] = interpolate(coordinates[start], coordinates[end]);
                        let curved = patch.evaluate(u, v);
                        let straight = interpolate(corners[start], corners[end]);
                        maximum_error = maximum_error.max((curved[0] - straight[0]).hypot(curved[1] - straight[1]));
                    }
                }

                // Texture coordinates stay in the original full tile, including across cell boundaries.
                let texture = coordinates.map(|point| point.map(|value| value * TILE_EDGE_TEXELS as f64));
                for indices in [[0, 1, 2], [0, 2, 3]] {
                    let screen = indices.map(|index| corners[index]);
                    let Some(triangle) = Triangle::new(screen, indices.map(|index| texture[index])) else { mesh.clear(); return false; };
                    mesh.push(triangle);
                }
            }
        }
        if maximum_error <= CURVED_PATCH_FLATNESS_PIXELS || divisions >= CURVED_PATCH_MAX_SUBDIVISIONS { return true; }
        divisions = (divisions * 2).min(CURVED_PATCH_MAX_SUBDIVISIONS);
    }
}

fn cross(origin: Point, first: Point, second: Point) -> f64 {
    // Signed twice-area also identifies which side of an oriented edge a point lies on.
    (first[0] - origin[0]) * (second[1] - origin[1]) - (first[1] - origin[1]) * (second[0] - origin[0])
}

/// Temporary pixels for one region's clipped screen bounding box. No data survives a frame. Accumulating all
/// triangle fragments before blending avoids dark seams and double source-over along subdivision boundaries.
#[derive(Default)]
pub(super) struct RegionPixels {
    pixels: Vec<[f64; 4]>,
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    sky: Option<SkyArea>,
}

impl RegionPixels {
    pub(super) fn prepare(&mut self, mesh: &[Triangle], width: usize, height: usize, sky: SkyArea) {
        // Clip allocation to this region's visible rectangle, reusing its capacity for following regions.
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in mesh.iter().flat_map(|triangle| triangle.screen) {
            for axis in 0..2 { minimum[axis] = minimum[axis].min(point[axis]); maximum[axis] = maximum[axis].max(point[axis]); }
        }
        let left = (minimum[0] - 0.5).ceil().max(0.0);
        let top = (minimum[1] - 0.5).ceil().max(0.0);
        let right = (maximum[0] + 0.5).floor().min((width - 1) as f64);
        let bottom = (maximum[1] + 0.5).floor().min((height - 1) as f64);
        self.width = if left <= right { (right - left) as usize + 1 } else { 0 };
        self.height = if top <= bottom { (bottom - top) as usize + 1 } else { 0 };
        self.left = left as usize;
        self.top = top as usize;
        self.sky = Some(sky);
        self.pixels.clear();
        self.pixels.resize(self.width * self.height, [0.0; 4]);
    }

    pub(super) fn draw_triangle(&mut self, triangle: &Triangle, sampler: &TileSampler, texels: &[Texel]) {
        // Scan only intersecting row spans. Fully covered pixels never enter the polygon clipper.
        if self.width == 0 || self.height == 0 { return; }
        let sky = self.sky.unwrap();
        super::triangle_rows::scan_triangle_rows(triangle.screen, self.left, self.top, self.width, self.height, sky, |span| {
            let footprint = triangle.mapping.footprint_of([1.0; 2]);
            for column in span.first..=span.last {
                let pixel = [column as f64, span.row as f64];
                let whole = span.is_whole(column);
                let whole_position = triangle.mapping.texture_position(pixel);
                if !whole && !sampler.may_cover_light(whole_position, footprint) { continue; }
                let fragment = if whole {
                    PixelFragment { coverage: 1.0, center: pixel, size: [1.0; 2] }
                } else {
                    let Some(fragment) = clip_outline_pixel(triangle.screen, pixel) else { continue; };
                    fragment
                };

                // Keep sampling coordinates and summation order unchanged for comparison with the reference.
                let position = if whole { whole_position } else { triangle.mapping.texture_position(fragment.center) };
                let footprint = if whole { footprint } else { triangle.mapping.footprint_of(fragment.size) };
                let Some(rgba) = sampler.sample(texels, position, footprint) else { continue; };
                let index = (span.row - self.top) * self.width + column - self.left;
                for (sum, value) in self.pixels[index].iter_mut().zip(rgba) { *sum += f64::from(value) * fragment.coverage; }
            }
        });
    }

    #[cfg(test)]
    fn draw_triangle_reference(&mut self, triangle: &Triangle, sampler: &TileSampler, texels: &[Texel]) {
        // Restrict work to the triangle's rectangle inside this region's screen rectangle.
        if self.width == 0 || self.height == 0 { return; }
        let minimum: Point = std::array::from_fn(|axis| triangle.screen.iter().map(|point| point[axis]).fold(f64::INFINITY, f64::min));
        let maximum: Point = std::array::from_fn(|axis| triangle.screen.iter().map(|point| point[axis]).fold(f64::NEG_INFINITY, f64::max));
        let left = (minimum[0] - 0.5).ceil().max(self.left as f64) as usize;
        let right = (maximum[0] + 0.5).floor().min((self.left + self.width - 1) as f64) as usize;
        let top = (minimum[1] - 0.5).ceil().max(self.top as f64) as usize;
        let bottom = (maximum[1] + 0.5).floor().min((self.top + self.height - 1) as f64) as usize;
        let sky = self.sky.unwrap();
        let radius = (sky.size as f64 - 1.0) * 0.5;
        let center = [sky.x as f64 + radius, sky.y as f64 + radius];

        // Integrate triangle fragments rather than blending adjacent triangles as overlapping images.
        for row in top..=bottom {
            for column in left..=right {
                if (column as f64 - center[0]).powi(2) + (row as f64 - center[1]).powi(2) > radius * radius + 1e-9 { continue; }
                let Some(fragment) = clip_pixel(triangle.screen, [column as f64, row as f64]) else { continue; };
                let position = triangle.mapping.texture_position(fragment.center);
                let footprint = triangle.mapping.footprint_of(fragment.size);
                let Some(rgba) = sampler.sample(texels, position, footprint) else { continue; };
                let index = (row - self.top) * self.width + column - self.left;
                for (sum, value) in self.pixels[index].iter_mut().zip(rgba) { *sum += f64::from(value) * fragment.coverage; }
            }
        }
    }

    pub(super) fn composite(&self, frame: &mut [Texel], width: usize) {
        // Blend the completed region once per pixel, keeping the existing order between different regions.
        for (index, rgba) in self.pixels.iter().enumerate() {
            if rgba[3] == 0.0 { continue; }
            #[cfg(feature = "pixel-debug")]
            crate::pixel_debug::check_pixel("curved region fragments", *rgba, 1.0);
            let scale = 1.0 / rgba[3].max(1.0); // absorb final-bit accumulation noise together in RGB and opacity
            let pixel = (self.top + index / self.width) * width + self.left + index % self.width;
            blend_star(&mut frame[pixel], [rgba[0], rgba[1], rgba[2]].map(|value| (value * scale) as f32), rgba[3].min(1.0) as f32);
        }
    }
}

pub(super) struct PixelFragment { coverage: f64, center: Point, size: Point }

/// Exact triangle / pixel intersection, in pixel-relative coordinates for stable area calculations.
#[cfg(test)]
pub(super) fn clip_pixel(triangle: [Point; 3], pixel: Point) -> Option<PixelFragment> {
    // Fully covered pixels avoid polygon clipping; this is the common case for large projected tiles.
    let pixel_corners = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
    let winding = cross(triangle[0], triangle[1], triangle[2]).signum();
    let whole = (0..3).all(|edge| pixel_corners.iter().all(|offset| {
        cross(triangle[edge], triangle[(edge + 1) % 3], [pixel[0] + offset[0], pixel[1] + offset[1]]) * winding >= 0.0
    }));
    if whole { return Some(PixelFragment { coverage: 1.0, center: pixel, size: [1.0; 2] }); }

    clip_outline_pixel(triangle, pixel)
}

/// Clip an outline pixel after the row scanner has already handled fully covered pixels.
fn clip_outline_pixel(triangle: [Point; 3], pixel: Point) -> Option<PixelFragment> {
    // Clip the triangle against all four pixel edges using fixed stack buffers.
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
                    vertices[written] = std::array::from_fn(|coordinate| previous[coordinate] + fraction * (point[coordinate] - previous[coordinate]));
                    written += 1;
                }
                if distance >= 0.0 { vertices[written] = point; written += 1; }
                previous = point;
                previous_distance = distance;
            }
            count = written;
            if count < 3 { return None; }
        }
    }

    // Use the polygon's bounding rectangle for the approximate texture footprint, and its exact covered area.
    let mut twice_area = 0.0;
    let mut minimum = [0.5_f64; 2];
    let mut maximum = [-0.5_f64; 2];
    let mut previous = vertices[count - 1];
    for point in vertices[..count].iter().copied() {
        twice_area += previous[0] * point[1] - previous[1] * point[0];
        for axis in 0..2 { minimum[axis] = minimum[axis].min(point[axis]); maximum[axis] = maximum[axis].max(point[axis]); }
        previous = point;
    }
    let coverage = (twice_area.abs() * 0.5).min(1.0);
    if coverage == 0.0 { return None; }
    Some(PixelFragment {
        coverage,
        center: std::array::from_fn(|axis| pixel[axis] + (minimum[axis] + maximum[axis]) * 0.5),
        size: std::array::from_fn(|axis| maximum[axis] - minimum[axis]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tile_sampling::lit_rows_of;

    #[test]
    fn row_spans_match_the_original_clipper_for_sparse_dense_and_thin_triangles() {
        // Keep the old drawing loop as an independent oracle for fractional edges, both windings and clipping.
        let sky = SkyArea { x: 3, y: 1, size: 47 };
        let mut random = 12345_u64;
        let mut next = || {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            (random >> 32) as f64 / u32::MAX as f64
        };
        let mut triangles = vec![
            [[25.8, 23.95], [26.2, 23.95], [26.2, 24.05]],
            [[24.0, 24.0], [26.0, 22.0], [28.0, 24.0]],
            [[-100.0, -100.0], [200.0, -100.0], [10.0, 200.0]],
            [[3.5, 3.5], [30.5, 3.5], [30.5, 30.5]],
        ];
        for _ in 0..200 {
            triangles.push(std::array::from_fn(|_| [next() * 100.0 - 25.0, next() * 100.0 - 25.0]));
        }
        let edge = TILE_EDGE_TEXELS;
        for sparse in [false, true] {
            let texels: Vec<_> = (0..edge * edge).map(|index| {
                let alpha = if sparse && index % 31 != 0 { 0.0 } else { (index % 17 + 1) as f32 / 18.0 };
                Texel { rgb: [alpha, alpha * 0.5, alpha * 0.25], opacity: alpha }
            }).collect();
            let mut sampler = TileSampler::new(edge);
            sampler.prepare(&lit_rows_of(&texels, edge));
            for corners in &triangles {
                for order in [[0, 1, 2], [2, 1, 0]] {
                    let screen = order.map(|index| corners[index]);
                    let texture = [[0.0, 0.0], [edge as f64, 0.0], [edge as f64, edge as f64]];
                    let Some(triangle) = Triangle::new(screen, texture) else { continue; };
                    let mut actual = RegionPixels::default();
                    let mut expected = RegionPixels::default();
                    actual.prepare(std::slice::from_ref(&triangle), 53, 51, sky);
                    expected.prepare(std::slice::from_ref(&triangle), 53, 51, sky);
                    actual.draw_triangle(&triangle, &sampler, &texels);
                    expected.draw_triangle_reference(&triangle, &sampler, &texels);
                    for (pixel, (actual, expected)) in actual.pixels.iter().zip(&expected.pixels).enumerate() {
                        for channel in 0..4 {
                            assert!((actual[channel] - expected[channel]).abs() < 1e-9,
                                "sparse={sparse} screen={screen:?} pixel={pixel} channel={channel}: {actual:?} != {expected:?}");
                        }
                    }
                }
            }
        }
    }
}
