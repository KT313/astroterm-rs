//! Temporary area filter for one merged tile. A summed-area table makes rectangular texture averages cheap.
//! It is rebuilt for each region and dropped after the frame, not retained as another rendering cache.
use crate::model::Texel;

pub(super) struct TileSampler {
    edge: usize,
    sums: Vec<[f64; 4]>,
}

impl TileSampler {
    pub(super) fn new(edge: usize) -> Self {
        // Include an empty top row and left column so every rectangle has four prefix-sum corners.
        Self { edge, sums: vec![[0.0; 4]; (edge + 1) * (edge + 1)] }
    }

    pub(super) fn prepare(&mut self, texels: &[Texel]) {
        // Replace the previous tile's prefix sums using its already-premultiplied RGB and opacity.
        assert_eq!(texels.len(), self.edge * self.edge);
        let stride = self.edge + 1;
        self.sums.fill([0.0; 4]);
        for row in 0..self.edge {
            let mut row_sum = [0.0; 4];
            for column in 0..self.edge {
                let texel = texels[row * self.edge + column];
                #[cfg(feature = "pixel-debug")]
                texel.check("whole tile filter input", 1.0);
                let rgba = [texel.rgb[0] as f64, texel.rgb[1] as f64, texel.rgb[2] as f64, texel.opacity as f64];
                let above = self.sums[row * stride + column + 1];
                for channel in 0..4 {
                    row_sum[channel] += rgba[channel];
                    self.sums[(row + 1) * stride + column + 1][channel] = above[channel] + row_sum[channel];
                }
            }
        }
    }

    /// Average a box centered at `position`, in texture coordinates from 0 to edge.
    /// The footprint is an axis-aligned approximation of the screen pixel in texture space. At magnification,
    /// a one-texel-wide box provides bilinear filtering instead of selecting one texel abruptly.
    pub(super) fn sample(&self, position: [f64; 2], footprint: [f64; 2]) -> [f64; 4] {
        // Clamp at the texture edges, where partially covered screen pixels may have centers outside the tile.
        let edge = self.edge as f64;
        let center = position.map(|value| value.clamp(0.0, edge));
        let half_width = footprint[0].max(1.0) * 0.5;
        let half_height = footprint[1].max(1.0) * 0.5;
        let left = (center[0] - half_width).max(0.0);
        let right = (center[0] + half_width).min(edge);
        let top = (center[1] - half_height).max(0.0);
        let bottom = (center[1] + half_height).min(edge);

        // Subtract the surrounding prefix sums to get the box's total, then divide by its area.
        let top_left = self.integral(left, top);
        let top_right = self.integral(right, top);
        let bottom_left = self.integral(left, bottom);
        let bottom_right = self.integral(right, bottom);
        let area = (right - left) * (bottom - top);
        let rgba = std::array::from_fn(|channel| {
            let value = (bottom_right[channel] - top_right[channel] - bottom_left[channel] + top_left[channel]) / area;
            #[cfg(feature = "pixel-debug")]
            assert!(value.is_finite() && value >= -1e-9, "whole tile filter: invalid channel {channel} = {value}");
            value.max(0.0) // Prefix-sum subtraction can produce a tiny negative rounding residual in an empty box.
        });
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("whole tile filter output", rgba, 1.0);
        rgba
    }

    /// Integral from the top-left texture boundary to a fractional coordinate. Within one texel, the integral
    /// is bilinear, so four neighbouring prefix sums give the exact rectangular area under constant texels.
    fn integral(&self, x: f64, y: f64) -> [f64; 4] {
        // Locate the surrounding prefix-sum entries, including the right and bottom texture boundaries.
        let column = (x.floor() as usize).min(self.edge - 1);
        let row = (y.floor() as usize).min(self.edge - 1);
        let fraction_x = x - column as f64;
        let fraction_y = y - row as f64;
        let stride = self.edge + 1;
        let index = row * stride + column;

        // Interpolate the integral, not the original texel colors.
        std::array::from_fn(|channel| {
            let top = self.sums[index][channel] * (1.0 - fraction_x) + self.sums[index + 1][channel] * fraction_x;
            let bottom = self.sums[index + stride][channel] * (1.0 - fraction_x) + self.sums[index + stride + 1][channel] * fraction_x;
            top * (1.0 - fraction_y) + bottom * fraction_y
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduction_includes_a_bright_texel_away_from_the_sample_center() {
        let mut texels = vec![Texel::default(); 16];
        texels[0] = Texel { rgb: [1.0, 0.5, 0.25], opacity: 1.0 };
        let mut sampler = TileSampler::new(4);
        sampler.prepare(&texels);
        assert_eq!(sampler.sample([2.0, 2.0], [4.0, 4.0]), [1.0 / 16.0, 0.5 / 16.0, 0.25 / 16.0, 1.0 / 16.0]);
    }

    #[test]
    fn fractional_boxes_match_direct_texel_area_averages() {
        let texels: Vec<_> = (0..16).map(|index| {
            let value = index as f32 / 16.0;
            Texel { rgb: [value, value * 0.5, 0.0], opacity: value }
        }).collect();
        let mut sampler = TileSampler::new(4);
        sampler.prepare(&texels);
        for (position, footprint) in [([1.7, 2.2], [2.3, 1.8]), ([0.0, 4.0], [1.0, 1.0]), ([2.0, 2.0], [0.1, 0.1])] {
            let [x, y] = position;
            let [width, height]: [f64; 2] = footprint;
            let (left, right) = ((x - width.max(1.0) / 2.0).max(0.0), (x + width.max(1.0) / 2.0).min(4.0));
            let (top, bottom) = ((y - height.max(1.0) / 2.0).max(0.0), (y + height.max(1.0) / 2.0).min(4.0));
            let mut expected = [0.0; 4];
            for row in 0..4 {
                for column in 0..4 {
                    let area = (right.min(column as f64 + 1.0) - left.max(column as f64)).max(0.0)
                        * (bottom.min(row as f64 + 1.0) - top.max(row as f64)).max(0.0);
                    let texel = texels[row * 4 + column];
                    for (sum, value) in expected.iter_mut().zip(texel.rgb.into_iter().chain([texel.opacity])) {
                        *sum += value as f64 * area / ((right - left) * (bottom - top));
                    }
                }
            }
            for (actual, expected) in sampler.sample(position, footprint).into_iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-12);
            }
        }
        sampler.prepare(&vec![Texel::default(); 16]);
        assert_eq!(sampler.sample([2.0, 2.0], [4.0, 4.0]), [0.0; 4]);
    }
}
