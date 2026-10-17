//! Four independent pixels per SSE2 batch. The rendering formula is shared with the scalar compositor;
//! only its execution is grouped. All loads/stores are unaligned, and Texel fields are read explicitly.
use std::arch::x86_64::*;
use crate::constants::{MIN_STAR_PIXEL_OPACITY, PIXEL_BACKGROUND_RGBA};
use crate::model::Texel;
use super::composition::compose_scalar_pixels;

pub(super) fn compose_pixels(stars: &[Texel], overlay: &[u8], output: &mut [u8]) {
    // Validate slice lengths once; complete chunks below provide the SIMD kernel's required bounds.
    assert_eq!(overlay.len(), stars.len() * 4);
    assert_eq!(output.len(), overlay.len());
    let batched_pixels = stars.len() / 4 * 4;
    let batched_bytes = batched_pixels * 4;
    for ((target, stars), overlay) in output[..batched_bytes].chunks_exact_mut(16)
        .zip(stars[..batched_pixels].chunks_exact(4)).zip(overlay[..batched_bytes].chunks_exact(16))
    {
        // SAFETY: x86-64 guarantees SSE2. Each chunk contains four stars or sixteen readable/writable
        // bytes; shared inputs and mutable output cannot alias. The kernel never reads Texel padding.
        unsafe { compose_four_pixels(stars, overlay, target); }
    }

    // Arbitrary image dimensions may leave fewer than four pixels. Keep them on the portable path.
    compose_scalar_pixels(&stars[batched_pixels..], &overlay[batched_bytes..], &mut output[batched_bytes..]);
}

/// Requires four stars, sixteen overlay bytes, sixteen output bytes, and SSE2 (the x86-64 baseline).
#[inline(always)]
unsafe fn compose_four_pixels(stars: &[Texel], overlay: &[u8], output: &mut [u8]) {
    // SAFETY: the only caller supplies complete chunks on x86-64. All pointer operations below use
    // unaligned sixteen-byte accesses within those chunks. Arithmetic helpers require only SSE2.
    unsafe {
        // Group the four red values together, then green, blue and opacity. No struct-layout cast is used.
        let channels = transpose_pixels(std::array::from_fn(|index| {
            let star = stars[index];
            _mm_setr_ps(star.rgb[0], star.rgb[1], star.rgb[2], star.opacity)
        }));
        let zero = _mm_setzero_ps();
        let one = _mm_set1_ps(1.0);
        let full = _mm_set1_ps(255.0);
        let minimum = _mm_set1_ps(MIN_STAR_PIXEL_OPACITY);
        let active = _mm_cmpgt_ps(channels[3], zero);
        let overlay_bytes = _mm_loadu_si128(overlay.as_ptr().cast());
        let empty_overlay = _mm_movemask_epi8(_mm_cmpeq_epi8(overlay_bytes, _mm_setzero_si128())) == 0xffff;

        // A wholly blank batch needs only a background store, with the same always-opaque output alpha.
        if _mm_movemask_ps(active) == 0 && empty_overlay {
            let background = [PIXEL_BACKGROUND_RGBA[0], PIXEL_BACKGROUND_RGBA[1], PIXEL_BACKGROUND_RGBA[2], 255];
            _mm_storeu_si128(output.as_mut_ptr().cast(), _mm_set1_epi32(u32::from_le_bytes(background) as i32));
            return;
        }

        // Select each pixel's scale/floor just as the scalar branches do. Ordinary pixels need no division.
        let saturated = _mm_cmpgt_ps(channels[3], one);
        let faint = _mm_cmplt_ps(channels[3], minimum);
        let needs_division = _mm_and_ps(active, _mm_or_ps(saturated, faint));
        let scale = if _mm_movemask_ps(needs_division) != 0 {
            let numerator = choose_lanes(saturated, full, _mm_set1_ps(255.0 * MIN_STAR_PIXEL_OPACITY));
            choose_lanes(needs_division, _mm_div_ps(numerator, channels[3]), full)
        } else {
            full
        };
        let floor = choose_lanes(saturated, one, choose_lanes(faint, minimum, channels[3]));
        let background_share = _mm_sub_ps(one, floor);

        // Keep multiply and add separate: fused operations or reciprocal approximations could change bytes.
        let mut rgb = std::array::from_fn::<_, 3, _>(|channel| {
            let background = _mm_set1_ps(PIXEL_BACKGROUND_RGBA[channel] as f32);
            let color = _mm_add_ps(_mm_mul_ps(channels[channel], scale), _mm_mul_ps(background, background_share));
            choose_lanes(active, _mm_min_ps(color, full), background)
        });

        // Most overlay batches are transparent. Expand their bytes only when blending is necessary.
        if !empty_overlay {
            let colors = unpack_overlay(overlay_bytes);
            let keep = _mm_sub_ps(one, _mm_div_ps(colors[3], full));
            for (color, overlay_color) in rgb.iter_mut().zip(&colors[..3]) {
                *color = _mm_add_ps(*overlay_color, _mm_mul_ps(*color, keep));
            }
        }

        // Match rounded, saturating Rust float-to-u8 conversion. MAX with zero also maps NaN to zero;
        // the bounded values fit i32, so truncation is safe. Each integer lane becomes one RGBA pixel.
        let bytes = rgb.map(|color| {
            let rounded = _mm_add_ps(color, _mm_set1_ps(0.5));
            _mm_cvttps_epi32(_mm_min_ps(_mm_max_ps(rounded, zero), full))
        });
        let red_green = _mm_or_si128(bytes[0], _mm_slli_epi32::<8>(bytes[1]));
        let blue_alpha = _mm_or_si128(_mm_slli_epi32::<16>(bytes[2]), _mm_set1_epi32(0xff00_0000_u32 as i32));
        _mm_storeu_si128(output.as_mut_ptr().cast(), _mm_or_si128(red_green, blue_alpha));
    }
}

/// Turn four RGBA pixel vectors into four vectors of corresponding channels. Requires SSE2.
#[inline(always)]
unsafe fn transpose_pixels(pixels: [__m128; 4]) -> [__m128; 4] {
    // SAFETY: callers run only on x86-64. These register operations access no memory.
    unsafe {
        let first_red_green = _mm_unpacklo_ps(pixels[0], pixels[1]);
        let first_blue_alpha = _mm_unpackhi_ps(pixels[0], pixels[1]);
        let last_red_green = _mm_unpacklo_ps(pixels[2], pixels[3]);
        let last_blue_alpha = _mm_unpackhi_ps(pixels[2], pixels[3]);
        [
            _mm_movelh_ps(first_red_green, last_red_green),
            _mm_movehl_ps(last_red_green, first_red_green),
            _mm_movelh_ps(first_blue_alpha, last_blue_alpha),
            _mm_movehl_ps(last_blue_alpha, first_blue_alpha),
        ]
    }
}

/// Choose yes/no independently for each pixel using an all-ones/all-zeros comparison mask. Requires SSE2.
#[inline(always)]
unsafe fn choose_lanes(mask: __m128, yes: __m128, no: __m128) -> __m128 {
    // SAFETY: callers run only on x86-64; these operations select register bits without touching memory.
    unsafe { _mm_or_ps(_mm_and_ps(mask, yes), _mm_andnot_ps(mask, no)) }
}

/// Expand sixteen premultiplied RGBA bytes into float channel vectors for four pixels. Requires SSE2.
#[inline(always)]
unsafe fn unpack_overlay(bytes: __m128i) -> [__m128; 4] {
    // SAFETY: callers run only on x86-64. Unpacking zero-extends each byte, so conversion is exact.
    unsafe {
        let zero = _mm_setzero_si128();
        let low = _mm_unpacklo_epi8(bytes, zero);
        let high = _mm_unpackhi_epi8(bytes, zero);
        transpose_pixels([
            _mm_cvtepi32_ps(_mm_unpacklo_epi16(low, zero)),
            _mm_cvtepi32_ps(_mm_unpackhi_epi16(low, zero)),
            _mm_cvtepi32_ps(_mm_unpacklo_epi16(high, zero)),
            _mm_cvtepi32_ps(_mm_unpackhi_epi16(high, zero)),
        ])
    }
}
