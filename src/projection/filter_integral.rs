//! Summed-area filtering experiment. Kept test-only until complete-frame speed and memory measurements
//! justify production use. Failed queries fall back to the ordinary area integrator, never to a clamped sum.
use crate::model::Texel;

pub(super) struct FilterIntegral {
    edge: usize,
    values: Vec<[f64; 4]>,
    sum_error: [f64; 4],
}

impl FilterIntegral {
    pub(super) fn build(texels: &[Texel], edge: usize) -> Self {
        assert!(edge > 0);
        assert_eq!(texels.len(), edge * edge);
        let stride = edge + 1;
        let mut values = vec![[0.0_f64; 4]; stride * stride];

        // Every prefix uses only nonnegative additions: at most edge across a row and edge down columns.
        for row in 0..edge {
            let mut row_sum = [0.0; 4];
            for column in 0..edge {
                let texel = texels[row * edge + column];
                for (channel, value) in texel.rgb.into_iter().chain([texel.opacity]).enumerate() {
                    assert!(value.is_finite() && value >= 0.0);
                    row_sum[channel] += f64::from(value);
                    values[(row + 1) * stride + column + 1][channel] = values[row * stride + column + 1][channel] + row_sum[channel];
                }
            }
        }

        // Bound construction, interpolation and cancellation separately, using the tile total as an upper
        // bound on each nonnegative corner integral. gamma(n) = n*u/(1-n*u), u = EPSILON/2.
        // A prefix has <=2*edge additions; a bilinear corner needs <16 rounded operations per source term.
        // The signed four-corner sum adds <=3 operations and has sum of magnitudes <=4*corner_upper.
        let prefix_error = gamma(2 * edge);
        let interpolation_error = gamma(16);
        let corner_error = prefix_error + interpolation_error + prefix_error * interpolation_error;
        let subtraction_error = gamma(3);
        let error_factor = 4.0 * (corner_error + subtraction_error * (1.0 + corner_error));
        let total = values[stride * stride - 1];
        let sum_error = total.map(|value| {
            let true_total_upper = value / (1.0 - prefix_error);
            true_total_upper * error_factor * (1.0 + 32.0 * f64::EPSILON) // outward slack for this bound's own rounded arithmetic
        });
        Self { edge, values, sum_error }
    }

    /// None means use direct integration, either because there is no overlap or cancellation is too large.
    pub(super) fn try_sample(&self, position: [f64; 2], footprint: [f64; 2]) -> Option<[f32; 4]> {
        let [x, y] = position;
        let width = footprint[0].max(1.0);
        let height = footprint[1].max(1.0);
        let left = (x - width * 0.5).max(0.0);
        let right = (x + width * 0.5).min(self.edge as f64);
        let top = (y - height * 0.5).max(0.0);
        let bottom = (y + height * 0.5).min(self.edge as f64);
        if right <= left || bottom <= top { return None; }

        // Bilinear prefix interpolation integrates fractional texel coverage, including partial borders.
        let top_left = self.integrate_to(left, top);
        let top_right = self.integrate_to(right, top);
        let bottom_left = self.integrate_to(left, bottom);
        let bottom_right = self.integrate_to(right, bottom);
        let area = width * height;
        let mut rgba = [0.0; 4];
        for channel in 0..4 {
            let sum = bottom_right[channel] - top_right[channel] - bottom_left[channel] + top_left[channel];
            // Keep cancellation uncertainty below 1e-9 relative, leaving ample room for the final f32
            // rounding within the existing 1e-6 normal-value tolerance. Zero channels are exact when total=0.
            if sum < 0.0 || self.sum_error[channel] > 1e-9 * sum { return None; }
            rgba[channel] = (sum / area) as f32;
        }
        Some(rgba)
    }

    #[inline]
    fn integrate_to(&self, x: f64, y: f64) -> [f64; 4] {
        let column = (x as usize).min(self.edge - 1);
        let row = (y as usize).min(self.edge - 1);
        let fraction_x = x - column as f64;
        let fraction_y = y - row as f64;
        let weights = [(1.0 - fraction_x) * (1.0 - fraction_y), fraction_x * (1.0 - fraction_y),
            (1.0 - fraction_x) * fraction_y, fraction_x * fraction_y];
        let stride = self.edge + 1;
        let offsets = [row * stride + column, row * stride + column + 1,
            (row + 1) * stride + column, (row + 1) * stride + column + 1];
        let mut sum = [0.0; 4];
        for (offset, weight) in offsets.into_iter().zip(weights) {
            for (channel, value) in sum.iter_mut().enumerate() { *value += self.values[offset][channel] * weight; }
        }
        sum
    }

    pub(super) fn bytes(&self) -> usize { self.values.capacity() * std::mem::size_of::<[f64; 4]>() }
}

fn gamma(operations: usize) -> f64 {
    let error = operations as f64 * (f64::EPSILON * 0.5);
    assert!(error < 1.0);
    error / (1.0 - error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::tile_sampling::{TileSampler, lit_rows_of};

    #[test]
    fn accepted_queries_match_direct_boxes_and_tiny_borders_fall_back() {
        let edge = 64;
        let mut texels: Vec<_> = (0..edge * edge).map(|index| {
            let alpha = (index % 19 + 1) as f32 / 21.0;
            Texel { rgb: [alpha * 0.2, alpha * 0.7, 0.0], opacity: alpha }
        }).collect();
        let integral = FilterIntegral::build(&texels, edge);
        assert_eq!(integral.bytes(), 65 * 65 * 32);
        let mut direct = TileSampler::new(edge, &Default::default());
        direct.prepare(&lit_rows_of(&texels, edge), None);
        let mut accepted = 0;

        // Vary position and rectangular footprint, including clipped and entirely covering queries.
        for index in 0..10_000 {
            let position = [(index * 139 % 7100) as f64 / 100.0 - 3.0, (index * 503 % 7100) as f64 / 100.0 - 3.0];
            let footprint = [0.5 + (index % 97) as f64, 0.5 + (index % 83) as f64];
            let expected = direct.sample(&texels, position, footprint).unwrap_or([0.0; 4]);
            if let Some(actual) = integral.try_sample(position, footprint) {
                for channel in 0..4 {
                    assert!((actual[channel] - expected[channel]).abs() <= 1e-6 * expected[channel].abs().max(1e-30));
                }
                accepted += 1;
            }
        }
        assert!(accepted > 9000, "{accepted}");

        // Large bright prefixes must not leak into the dark interior or overwhelm its almost-zero borders.
        for row in 16..48 { for column in 16..48 { texels[row * edge + column] = Texel::default(); } }
        let integral = FilterIntegral::build(&texels, edge);
        direct.prepare(&lit_rows_of(&texels, edge), None);
        for epsilon in [0.0, 1e-12, 1e-9, 1e-6] {
            let position = [32.0; 2];
            let footprint = [32.0 + epsilon; 2];
            assert!(integral.try_sample(position, footprint).is_none());
            let result = direct.sample(&texels, position, footprint);
            if epsilon == 0.0 { assert_eq!(result, None); }
            else { assert!(result.unwrap()[3] > 0.0); }
        }
    }
}
