//! Remove the final image's constant alpha bytes before sending RGB to Kitty.

/// Reuse the RGB buffer and select the byte-shuffle kernel once per image, outside the pixel loop.
pub(super) fn strip_alpha(rgba: &[u8], rgb: &mut Vec<u8>) {
    // Keep diagnostics before conversion so removing alpha cannot hide a nonopaque input.
    #[cfg(feature = "pixel-debug")]
    {
        crate::pixel_debug::check_overlay("strip_alpha input", rgba);
        assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 255), "pixel-debug: strip_alpha requires opaque pixels");
    }

    // Size the destination once; ignore incomplete source pixels as the previous chunks_exact loop did.
    let pixel_count = rgba.len() / 4;
    let rgba = &rgba[..pixel_count * 4];
    rgb.resize(pixel_count * 3, 0);

    // SSSE3 is optional even on x86-64. Rust caches CPU detection; other CPUs use the portable loop.
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("ssse3") {
        // SAFETY: the CPU supports SSSE3. The kernel checks lengths before any pointer accesses.
        unsafe { copy_rgb_ssse3(rgba, rgb); }
        return;
    }
    copy_rgb_scalar(rgba, rgb);
}

/// Copy complete pixels without alpha, also handling the SIMD kernel's final zero to fifteen pixels.
fn copy_rgb_scalar(rgba: &[u8], rgb: &mut [u8]) {
    assert_eq!(rgba.len() % 4, 0);
    assert_eq!(rgb.len(), rgba.len() / 4 * 3);
    for (pixel, target) in rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
        target.copy_from_slice(&pixel[..3]);
    }
}

/// Pack sixteen RGBA pixels into forty-eight RGB bytes without changing any channel values.
///
/// # Safety
/// The caller must verify that the CPU supports SSSE3. Slice lengths are checked here.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn copy_rgb_ssse3(rgba: &[u8], rgb: &mut [u8]) {
    use std::arch::x86_64::*;

    // Each input register holds four pixels; shuffle their RGB bytes together and zero the last four bytes.
    assert_eq!(rgba.len() % 4, 0);
    assert_eq!(rgb.len(), rgba.len() / 4 * 3);
    let rgb_indices = _mm_setr_epi8(0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, -1, -1, -1, -1);
    let batch_count = rgba.len() / 64;
    for batch in 0..batch_count {
        // SAFETY: each batch reads exactly 64 source bytes and writes exactly 48 destination bytes.
        // The length checks and batch count keep every access within its slice. Unaligned access is allowed.
        unsafe {
            let source = rgba.as_ptr().add(batch * 64);
            let target = rgb.as_mut_ptr().add(batch * 48);
            let first = _mm_shuffle_epi8(_mm_loadu_si128(source.cast()), rgb_indices);
            let second = _mm_shuffle_epi8(_mm_loadu_si128(source.add(16).cast()), rgb_indices);
            let third = _mm_shuffle_epi8(_mm_loadu_si128(source.add(32).cast()), rgb_indices);
            let fourth = _mm_shuffle_epi8(_mm_loadu_si128(source.add(48).cast()), rgb_indices);

            // Join four twelve-byte RGB groups into three full registers, with no overlapping stores.
            let first_output = _mm_or_si128(first, _mm_slli_si128::<12>(second));
            let second_output = _mm_or_si128(_mm_srli_si128::<4>(second), _mm_slli_si128::<8>(third));
            let third_output = _mm_or_si128(_mm_srli_si128::<8>(third), _mm_slli_si128::<4>(fourth));
            _mm_storeu_si128(target.cast(), first_output);
            _mm_storeu_si128(target.add(16).cast(), second_output);
            _mm_storeu_si128(target.add(32).cast(), third_output);
        }
    }

    // A short final batch uses exact-size scalar copies, never a vector load beyond the image.
    copy_rgb_scalar(&rgba[batch_count * 64..], &mut rgb[batch_count * 48..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rgba(pixel_count: usize) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for index in 0..pixel_count {
            rgba.extend_from_slice(&[index as u8, (index / 3) as u8, (index * 73) as u8, 255]);
        }
        rgba
    }

    fn reference_rgb(rgba: &[u8]) -> Vec<u8> {
        let mut rgb = Vec::new();
        for pixel in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        rgb
    }

    #[test]
    fn conversion_reuses_storage_and_handles_resizes_empty_images_and_all_tails() {
        let mut rgb = Vec::with_capacity(4096 * 3);
        let allocation = rgb.as_ptr();
        for pixel_count in (0..=1025).rev() {
            let rgba = make_rgba(pixel_count);
            strip_alpha(&rgba, &mut rgb);
            assert_eq!(rgb, reference_rgb(&rgba), "{pixel_count} pixels");
            assert_eq!(rgb.as_ptr(), allocation);
        }
        strip_alpha(&make_rgba(5000), &mut rgb);
        assert_eq!(rgb, reference_rgb(&make_rgba(5000)));
    }

    #[test]
    fn scalar_and_simd_preserve_bytes_with_unaligned_slices_and_exact_size_outputs() {
        // Exercise every input/output alignment and every remainder without letting a store touch the guards.
        let source = make_rgba(300);
        for offset in 0..16 {
            for pixel_count in 0..=257 {
                let rgba = &source[offset..offset + pixel_count * 4];
                let expected = reference_rgb(rgba);
                let mut storage = vec![0xa5; offset + expected.len() + 16];
                let end = offset + expected.len();
                copy_rgb_scalar(rgba, &mut storage[offset..end]);
                assert_eq!(&storage[offset..end], expected);

                #[cfg(target_arch = "x86_64")]
                if std::is_x86_feature_detected!("ssse3") {
                    storage[offset..end].fill(0xa5);
                    // SAFETY: detection above confirms SSSE3 support; slices contain complete pixels.
                    unsafe { copy_rgb_ssse3(rgba, &mut storage[offset..end]); }
                    assert_eq!(&storage[offset..end], expected);
                }
                assert!(storage[..offset].iter().chain(&storage[end..]).all(|&byte| byte == 0xa5));
            }
        }
    }

    #[test]
    #[cfg(feature = "pixel-debug")]
    fn diagnostics_reject_nonopaque_pixels_before_the_simd_batches_or_tail_are_written() {
        for pixel_index in [0, 15, 16, 31, 32] {
            let mut rgba = make_rgba(33);
            rgba[pixel_index * 4 + 3] = 254;
            let mut rgb = vec![0xa5; 99];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| strip_alpha(&rgba, &mut rgb)));
            assert!(result.is_err());
            assert_eq!(rgb, vec![0xa5; 99]);
        }
    }

    #[test]
    fn kernels_reject_mismatched_lengths_before_writing() {
        let source = make_rgba(16);
        let mut output = [0xa5; 47];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| copy_rgb_scalar(&source, &mut output)));
        assert!(result.is_err());
        assert_eq!(output, [0xa5; 47]);

        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("ssse3") {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // SAFETY: detection above confirms SSSE3 support; invalid lengths must panic before access.
                unsafe { copy_rgb_ssse3(&source, &mut output); }
            }));
            assert!(result.is_err());
            assert_eq!(output, [0xa5; 47]);
        }
    }
}
