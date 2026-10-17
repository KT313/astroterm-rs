//! Convert floating-point stars and the premultiplied overlay into the final opaque RGBA image.
use crate::constants::{MIN_STAR_PIXEL_OPACITY, PIXEL_BACKGROUND_RGBA};
use crate::model::Texel;
use crate::projection::Frame;

/// Tone map stars over the background, blend the overlay, and round once to opaque 8-bit RGBA.
/// Normal x86-64 builds use four-pixel SSE2 batches. Other architectures and pixel-debug builds use the
/// scalar path; diagnostics retain every input/intermediate check and accumulated-light sample.
pub fn compose(frame: &Frame, output: &mut Vec<u8>) {
    // Keep the allocation and write directly into its pixels, including blank areas from previous frames.
    output.resize(frame.stars().len() * 4, 0);

    // SSE2 is guaranteed on x86-64. Checked builds deliberately keep the scalar diagnostic boundaries.
    #[cfg(all(target_arch = "x86_64", not(feature = "pixel-debug")))]
    super::composition_sse2::compose_pixels(frame.stars(), frame.overlay(), output);
    #[cfg(any(not(target_arch = "x86_64"), feature = "pixel-debug"))]
    compose_scalar_pixels(frame.stars(), frame.overlay(), output);
}

/// Portable compositor, also used for a SIMD batch's final one to three pixels.
pub(super) fn compose_scalar_pixels(stars: &[Texel], overlay: &[u8], output: &mut [u8]) {
    assert_eq!(overlay.len(), stars.len() * 4);
    assert_eq!(output.len(), overlay.len());
    for ((target, star), overlay) in output.chunks_exact_mut(4).zip(stars).zip(overlay.chunks_exact(4)) {
        target.copy_from_slice(&compose_pixel(star, overlay));
    }
}

fn compose_pixel(star: &Texel, overlay: &[u8]) -> [u8; 4] {
    // Validate the inputs before any shortcut or clamp could conceal an invalid channel.
    #[cfg(feature = "pixel-debug")]
    {
        star.check("compose star input", f64::INFINITY);
        crate::pixel_debug::record_accumulated([star.rgb[0] as f64, star.rgb[1] as f64, star.rgb[2] as f64, star.opacity as f64]);
        crate::pixel_debug::check_overlay("compose overlay input", overlay);
    }

    // Apply the existing faint-pixel floor and saturation policy without changing arithmetic order.
    let mut pixel = [PIXEL_BACKGROUND_RGBA[0] as f32, PIXEL_BACKGROUND_RGBA[1] as f32, PIXEL_BACKGROUND_RGBA[2] as f32];
    if star.opacity > 0.0 {
        let (scale, floor) = if star.opacity > 1.0 { (255.0 / star.opacity, 1.0) }
            else if star.opacity < MIN_STAR_PIXEL_OPACITY { (255.0 * MIN_STAR_PIXEL_OPACITY / star.opacity, MIN_STAR_PIXEL_OPACITY) }
            else { (255.0, star.opacity) };
        for (index, (channel, color)) in pixel.iter_mut().zip(star.rgb).enumerate() {
            let value = color * scale + *channel * (1.0 - floor);
            #[cfg(feature = "pixel-debug")]
            crate::pixel_debug::check_channel("compose tone mapping before clamp", index, value as f64, 255.0);
            let _ = index;
            *channel = value.min(255.0);
        }
    }

    // The overlay already contains its opacity-weighted colour; round only after blending it.
    let alpha = f32::from(overlay[3]) / 255.0;
    for (channel, &drawn) in pixel.iter_mut().zip(&overlay[..3]) {
        *channel = f32::from(drawn) + *channel * (1.0 - alpha);
    }
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_pixel("compose before u8 conversion", [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64, 255.0], 255.0);
    let [red, green, blue] = pixel.map(|channel| (channel + 0.5) as u8);
    [red, green, blue, 255]
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
