//! Screen-pixel coverage and tile composition for the whole-tile forward renderer.
use crate::model::Texel;
use crate::projection::SkyArea;
use crate::regions::blend_star;

pub(super) type Point = [f64; 2];

/// Per-frame scratch: accumulate a tile's coverage in f64, then touch only its painted pixels when composing
/// and clearing it. Adjacent texels and the two halves of one texel must not blend as separate opaque layers.
pub(super) struct TileRaster {
    pixels: Vec<[f64; 4]>,
    touched: Vec<usize>,
    width: usize,
    height: usize,
    sky: SkyArea,
}

impl TileRaster {
    pub(super) fn new(width: usize, height: usize, sky: SkyArea) -> Self {
        // Allocate one screen-sized accumulation buffer, reused for each region in this frame.
        Self { pixels: vec![[0.0; 4]; width * height], touched: Vec::new(), width, height, sky }
    }

    /// Accumulate fractional screen-pixel coverage, taking the color from the supplied texture sampler.
    pub(super) fn draw_textured_triangle(&mut self, triangle: [Point; 3], sample: impl Fn(Point) -> [f64; 4]) {
        // Limit the pixel loop to the triangle's bounding rectangle inside the frame.
        let minimum = [0, 1].map(|axis| triangle.iter().map(|point| point[axis]).fold(f64::INFINITY, f64::min));
        let maximum = [0, 1].map(|axis| triangle.iter().map(|point| point[axis]).fold(f64::NEG_INFINITY, f64::max));
        let (left, right) = ((minimum[0] - 0.5).ceil().max(0.0), (maximum[0] + 0.5).floor().min((self.width - 1) as f64));
        let (top, bottom) = ((minimum[1] - 0.5).ceil().max(0.0), (maximum[1] + 0.5).floor().min((self.height - 1) as f64));
        if left > right || top > bottom { return; }
        let radius = (self.sky.size as f64 - 1.0) * 0.5;
        let (cx, cy) = (self.sky.x as f64 + radius, self.sky.y as f64 + radius);

        // Sample only covered pixels inside the sky circle, accumulating both triangle halves before blending.
        for row in top as usize..=bottom as usize {
            for column in left as usize..=right as usize {
                if (column as f64 - cx).powi(2) + (row as f64 - cy).powi(2) > radius * radius + 1e-9 { continue; }
                let coverage = triangle_coverage(triangle, [column as f64, row as f64]);
                if coverage == 0.0 { continue; }
                let rgba = sample([column as f64, row as f64]);
                if rgba[3] == 0.0 { continue; }
                let index = row * self.width + column;
                if self.pixels[index][3] == 0.0 { self.touched.push(index); }
                for (sum, value) in self.pixels[index].iter_mut().zip(rgba) {
                    *sum += value * coverage;
                }
            }
        }
    }

    pub(super) fn composite(&mut self, frame: &mut [Texel]) {
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

    fn draw_triangle(raster: &mut TileRaster, triangle: [Point; 3], texel: Texel) {
        // Use a constant texture value to isolate coverage and composition in these tests.
        if texel.is_empty() { return; }
        let rgba = [texel.rgb[0] as f64, texel.rgb[1] as f64, texel.rgb[2] as f64, texel.opacity as f64];
        raster.draw_textured_triangle(triangle, |_| rgba);
    }

    fn raster() -> TileRaster {
        TileRaster { pixels: vec![[0.0; 4]; 9 * 9], touched: Vec::new(), width: 9, height: 9, sky: SkyArea { x: 0, y: 0, size: 9 } }
    }
    fn square(raster: &mut TileRaster, low: f64, high: f64, texel: Texel) {
        let corners = [[low, low], [high, low], [high, high], [low, high]];
        draw_triangle(raster, [corners[0], corners[1], corners[2]], texel);
        draw_triangle(raster, [corners[0], corners[2], corners[3]], texel);
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
            draw_triangle(&mut raster, triangle, red);
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
}
