//! Unit-size box filtering inside a tile. The four overlap weights are the bilinear weights;
//! keep the box-edge arithmetic so rounding at texel boundaries matches the general filter.
use crate::model::Texel;

/// Both coordinates must be in [0.5, edge - 0.5), so all four neighbours are valid tile indices.
/// Positions use edge coordinates: texel zero is centred at 0.5, not zero.
#[inline]
pub(super) fn sample_bilinear_interior(texels: &[Texel], edge: usize, position: [f64; 2]) -> Option<[f32; 4]> {
    // Find the first texel touched by the unit box and the boundary to its next neighbour.
    let [column, row] = position;
    let (left, top) = (column - 0.5, row - 0.5);
    let (first_column, first_row) = (left as usize, top as usize);
    let column_boundary = (first_column + 1) as f64;
    let row_boundary = (first_row + 1) as f64;

    // Use the actual rounded box edges, including zero weight when an edge exactly touches a texel.
    let (right, bottom) = (column + 0.5, row + 0.5);
    let left_width = right.min(column_boundary) - left;
    let right_width = (right - column_boundary).max(0.0);
    let top_height = bottom.min(row_boundary) - top;
    let bottom_height = (bottom - row_boundary).max(0.0);
    let weights = [left_width * top_height, right_width * top_height,
        left_width * bottom_height, right_width * bottom_height];

    // Accumulate the four pixels in the same row order as the box filter, without renormalizing.
    let first = first_row * edge + first_column;
    let neighbours = [texels[first], texels[first + 1], texels[first + edge], texels[first + edge + 1]];
    #[cfg(target_arch = "x86_64")]
    let rgba = accumulate_sse2(neighbours, weights);
    #[cfg(not(target_arch = "x86_64"))]
    let rgba = accumulate_scalar(neighbours, weights);

    // Check for empty support before converting, so a tiny positive contribution is not discarded.
    if rgba[3] == 0.0 { return None; }
    let rgba = rgba.map(|channel| channel as f32);
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_pixel("bilinear filter output", rgba.map(f64::from), 1.0);
    Some(rgba)
}

/// Portable fallback and independent accumulation reference for the SIMD tests.
#[cfg(any(not(target_arch = "x86_64"), test))]
#[inline]
fn accumulate_scalar(neighbours: [Texel; 4], weights: [f64; 4]) -> [f64; 4] {
    let mut rgba = [0.0_f64; 4];
    for (texel, weight) in neighbours.into_iter().zip(weights) {
        for (sum, channel) in rgba[..3].iter_mut().zip(texel.rgb) { *sum += f64::from(channel) * weight; }
        rgba[3] += f64::from(texel.opacity) * weight;
    }
    rgba
}

/// Accumulate independent RG and BA channel pairs in f64, retaining the scalar texel/operation order.
#[cfg(target_arch = "x86_64")]
#[inline]
fn accumulate_sse2(neighbours: [Texel; 4], weights: [f64; 4]) -> [f64; 4] {
    use std::arch::x86_64::*;

    // SAFETY: SSE2 is guaranteed on x86-64. Input fields are read normally, without casting Texel's
    // layout. The two unaligned stores each write two f64 values within the four-element result array.
    unsafe {
        let mut red_green = _mm_setzero_pd();
        let mut blue_alpha = _mm_setzero_pd();
        for (texel, weight) in neighbours.into_iter().zip(weights) {
            // Convert all four channels exactly before multiplication; no f32 accumulation or FMA.
            let channels = _mm_setr_ps(texel.rgb[0], texel.rgb[1], texel.rgb[2], texel.opacity);
            let red_green_values = _mm_cvtps_pd(channels);
            let blue_alpha_values = _mm_cvtps_pd(_mm_movehl_ps(channels, channels));
            let factor = _mm_set1_pd(weight);
            red_green = _mm_add_pd(red_green, _mm_mul_pd(red_green_values, factor));
            blue_alpha = _mm_add_pd(blue_alpha, _mm_mul_pd(blue_alpha_values, factor));
        }

        // Keep the f64 result until the caller checks empty support, then converts to f32 as before.
        let mut rgba = [0.0_f64; 4];
        _mm_storeu_pd(rgba.as_mut_ptr(), red_green);
        _mm_storeu_pd(rgba.as_mut_ptr().add(2), blue_alpha);
        rgba
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn simd_accumulation_matches_scalar_bits_including_tiny_and_zero_weights() {
        let mut seed = 19_u32;
        for index in 0..20_000 {
            let neighbours = std::array::from_fn(|corner| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let opacity = match (index + corner) % 5 {
                    0 => 0.0,
                    1 => f32::from_bits(1),
                    2 => f32::MIN_POSITIVE,
                    _ => (seed >> 8) as f32 / 16777215.0,
                };
                Texel { rgb: [opacity * 0.2, opacity * 0.7, opacity], opacity }
            });
            let x = (index % 1001) as f64 / 1000.0;
            let y = (index * 53 % 1001) as f64 / 1000.0;
            let weights = if index % 7 == 0 { [0.0, f64::MIN_POSITIVE, 0.0, 0.0] }
                else { [(1.0 - x) * (1.0 - y), x * (1.0 - y), (1.0 - x) * y, x * y] };
            assert_eq!(accumulate_sse2(neighbours, weights).map(f64::to_bits),
                accumulate_scalar(neighbours, weights).map(f64::to_bits));
        }
    }

    #[cfg(feature = "pixel-debug")]
    #[test]
    fn simd_sampling_keeps_the_existing_output_validation() {
        let texels = [Texel { rgb: [2.0, 0.0, 0.0], opacity: 1.0 }; 4];
        let failure = std::panic::catch_unwind(|| sample_bilinear_interior(&texels, 2, [1.0, 1.0])).unwrap_err();
        assert!(failure.downcast_ref::<String>().unwrap().contains("bilinear filter output"));
    }
}
