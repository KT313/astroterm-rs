//! Independent pre-optimization oracle plus scalar/SIMD parity and diagnostic checks.
use super::*;

fn compose_reference(frame: &Frame, out: &mut Vec<u8>) {
    let background = PIXEL_BACKGROUND_RGBA.map(f32::from);
    out.clear();
    out.reserve(frame.width() * frame.height() * 4);
    for (texel, overlay) in frame.stars().iter().zip(frame.overlay().chunks_exact(4)) {
        #[cfg(feature = "pixel-debug")]
        {
            texel.check("compose star input", f64::INFINITY);
            crate::pixel_debug::record_accumulated([texel.rgb[0] as f64, texel.rgb[1] as f64, texel.rgb[2] as f64, texel.opacity as f64]);
            crate::pixel_debug::check_overlay("compose overlay input", overlay);
        }
        let mut pixel = [background[0], background[1], background[2]];
        if texel.opacity > 0.0 {
            let (scale, floor) = if texel.opacity > 1.0 { (255.0 / texel.opacity, 1.0) }                // saturated: the straight colour
                else if texel.opacity < MIN_STAR_PIXEL_OPACITY { (255.0 * MIN_STAR_PIXEL_OPACITY / texel.opacity, MIN_STAR_PIXEL_OPACITY) } // raised to the floor, colour intact
                else { (255.0, texel.opacity) };
            for (index, (channel, color)) in pixel.iter_mut().zip(texel.rgb).enumerate() {
                let value = color * scale + *channel * (1.0 - floor);
                #[cfg(feature = "pixel-debug")]
                crate::pixel_debug::check_channel("compose tone mapping before clamp", index, value as f64, 255.0);
                let _ = index;
                *channel = value.min(255.0);
            }
        }
        let alpha = f32::from(overlay[3]) / 255.0;
        for (channel, &drawn) in pixel.iter_mut().zip(&overlay[..3]) { *channel = f32::from(drawn) + *channel * (1.0 - alpha); } // the overlay is premultiplied
        #[cfg(feature = "pixel-debug")]
        crate::pixel_debug::check_pixel("compose before u8 conversion", [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64, 255.0], 255.0);
        out.extend(pixel.map(|channel| (channel + 0.5) as u8));
        out.push(255);
    }
}

fn check_backends(frame: &Frame) {
    let mut expected = Vec::new();
    compose_reference(frame, &mut expected);

    // All callers overwrite their entire output, even if its prior size/content differs.
    let mut actual = vec![0xa5; expected.len() + 9];
    compose(frame, &mut actual);
    assert_eq!(actual, expected);
    compose_scalar_pixels(frame.stars(), frame.overlay(), &mut actual);
    assert_eq!(actual, expected);

    // Exercise unaligned byte slices and verify the vector stores never touch either surrounding guard.
    #[cfg(target_arch = "x86_64")]
    {
        let mut overlay = vec![0x5a];
        overlay.extend_from_slice(frame.overlay());
        let mut guarded = vec![0xa5; expected.len() + 3];
        super::super::composition_sse2::compose_pixels(frame.stars(), &overlay[1..], &mut guarded[1..expected.len() + 1]);
        assert_eq!(&guarded[1..expected.len() + 1], expected);
        assert_eq!(guarded[0], 0xa5);
        assert_eq!(&guarded[expected.len() + 1..], [0xa5, 0xa5]);
    }
}

#[test]
fn composition_matches_reference_at_thresholds_with_overlays_and_partial_batches() {
    let opacities = [0.0, 1e-24, MIN_STAR_PIXEL_OPACITY.next_down().max(0.0),
        MIN_STAR_PIXEL_OPACITY, MIN_STAR_PIXEL_OPACITY.next_up(),
        1.0, f32::from_bits(1.0_f32.to_bits() + 1), 10.0, 200.0];
    let mut seed = 17_u32;
    for width in [0, 1, 2, 3, 4, 5, 6, 7, 8, 17, 65539] {
        let mut frame = Frame::new(width, 1, width.min(1), (10, 20));
        for (index, star) in frame.stars_mut().iter_mut().enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let fraction = (seed >> 8) as f32 / 16777215.0;
            let opacity = opacities[index % opacities.len()];
            *star = Texel { rgb: [fraction * opacity, 0.3 * opacity, 0.7 * opacity], opacity };
        }
        for (index, pixel) in frame.overlay_and_text().0.chunks_exact_mut(4).enumerate() {
            let alpha = [0, 255, 128, (index % 256) as u8][index % 4];
            pixel.copy_from_slice(&[alpha, alpha / 2, alpha / 3, alpha]);
        }
        check_backends(&frame);
    }
}

#[test]
fn composition_reuses_output_and_clears_old_pixels_after_empty_frames_and_resizes() {
    let mut frame = Frame::square(17);
    frame.stars_mut().fill(Texel { rgb: [0.5, 0.2, 0.1], opacity: 0.7 });
    let mut output = Vec::new();
    compose(&frame, &mut output);
    let address = output.as_ptr();
    let capacity = output.capacity();
    for size in [17, 3, 0, 1, 17] {
        frame.resize(size, size, size);
        frame.stars_mut().fill(Texel::default());
        frame.overlay_and_text().0.fill(0);
        compose(&frame, &mut output);
        let mut expected = Vec::new();
        compose_reference(&frame, &mut expected);
        assert_eq!(output, expected);
        assert_eq!(output.as_ptr(), address);
        assert_eq!(output.capacity(), capacity);
    }
}

// Normal rendering retains the existing saturating conversion semantics even for malformed floats.
// Checked builds deliberately reject these inputs before composition instead.
#[cfg(not(feature = "pixel-debug"))]
#[test]
fn unchecked_composition_matches_reference_for_subnormal_and_nonfinite_inputs() {
    let mut frame = Frame::new(65539, 1, 1, (10, 20));
    let opacities = [f32::from_bits(1), f32::MIN_POSITIVE, f32::INFINITY, f32::NAN,
        -1.0, f32::NEG_INFINITY, -0.0, 0.0, 0.05, 1.0, 10.0, f32::MAX];
    let mut seed = 23_u32;
    for (index, star) in frame.stars_mut().iter_mut().enumerate() {
        let opacity = opacities[index % opacities.len()];
        let rgb = std::array::from_fn(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            f32::from_bits(seed)
        });
        *star = Texel { rgb, opacity };
    }
    for (index, pixel) in frame.overlay_and_text().0.chunks_exact_mut(4).enumerate() {
        let alpha = [0, 255, 128, 32][index % 4];
        pixel.copy_from_slice(&[alpha / 2, alpha, alpha / 3, alpha]);
    }
    check_backends(&frame);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn simd_rejects_mismatched_slices_before_loading_or_storing() {
    let stars = [Texel::default(); 4];
    assert!(std::panic::catch_unwind(|| {
        super::super::composition_sse2::compose_pixels(&stars, &[0; 15], &mut [0; 16]);
    }).is_err());
    assert!(std::panic::catch_unwind(|| {
        super::super::composition_sse2::compose_pixels(&stars, &[0; 16], &mut [0; 15]);
    }).is_err());
}

#[cfg(feature = "pixel-debug")]
#[test]
fn checked_composition_rejects_invalid_stars_even_when_hidden_by_opaque_overlay() {
    for index in 0..8 {
        let mut frame = Frame::square(3);
        frame.stars_mut()[index] = Texel { rgb: [1.1, 0.0, 0.0], opacity: 1.0 };
        frame.overlay_and_text().0.fill(255);
        let failure = std::panic::catch_unwind(|| compose(&frame, &mut Vec::new())).unwrap_err();
        assert!(failure.downcast_ref::<String>().unwrap().contains("compose star input"));
    }
}
