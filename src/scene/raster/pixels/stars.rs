//! Fixed four-pixel stars with straight RGB and independent opacity. No drawing-library blending is used here.
use crate::constants::{DYNAMIC_NAME_COUNT, MAX_IMAGE_PIXELS, MIN_STAR_PIXEL_OPACITY,
    STAR_OPACITY_REFERENCE_MAGNITUDE, STAR_OPACITY_MAGNITUDE_SCALE, MIN_FOV_DEGREES,
    STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES, STAR_BRIGHTNESS_ZOOM_POWER};
use crate::model::{Cell, PixelStarKey, ProjectedSky, ProjectionViewport, RenderOptions, StarPixel};
use tiny_skia::Pixmap;

/// Validate the whole footprint once during preparation; drawing uses only accepted coordinates.
pub(crate) fn pixel_star_fits((y, x): Cell, viewport: ProjectionViewport) -> bool {
    x > 0 && y > 0 && (x as usize) < viewport.width && (y as usize) < viewport.height
}

/// Walk from the bright end until enough drawable stars are found, skipping omitted edge stars.
/// A small stack array retains label overwrite order without a candidate allocation or full-list preparation pass.
pub(crate) fn select_pixel_star_labels(options: &RenderOptions, sky: &ProjectedSky<'_>) -> impl ExactSizeIterator<Item = usize> + DoubleEndedIterator {
    let mut indices = [0; DYNAMIC_NAME_COUNT];
    let mut count = 0;
    if options.dynamic_names {
        for (index, entry) in sky.stars.iter().enumerate().rev() {
            if count == DYNAMIC_NAME_COUNT || entry.star.magnitude > options.magnitude_threshold { break; }
            if entry.cell.is_some_and(|cell| pixel_star_fits(cell, sky.viewport)) {
                indices[count] = index;
                count += 1;
            }
        }
    }
    indices.into_iter().take(count).rev()
}

pub(in crate::scene) fn initialize_star_layer(layer: &mut Vec<StarPixel>, viewport: ProjectionViewport) -> Option<()> {
    let count = viewport.width.checked_mul(viewport.height)?;
    if count == 0 || count > MAX_IMAGE_PIXELS { return None; }
    u32::try_from(viewport.width).ok()?;
    u32::try_from(viewport.height).ok()?;
    layer.clear();
    layer.try_reserve(count).ok()?;
    layer.resize(count, StarPixel::default()); // reuse capacity, but never retain the previous frame's colors
    Some(())
}

/// A visual compensation for fewer overlapping stars when zoomed in, independent of catalog brightness.
pub(in crate::scene) fn calculate_zoom_opacity_boost(fov_degrees: f64) -> f64 {
    let zoom = (STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES / fov_degrees.max(MIN_FOV_DEGREES)).max(1.0);
    zoom.powf(STAR_BRIGHTNESS_ZOOM_POWER)
}

fn calculate_star_opacity(magnitude: f64, zoom_boost: f64) -> f32 {
    let exponent = -STAR_OPACITY_MAGNITUDE_SCALE * (magnitude - STAR_OPACITY_REFERENCE_MAGNITUDE);
    (10_f64.powf(exponent) * zoom_boost).clamp(f64::from(f32::MIN_POSITIVE), 1.0) as f32 // keep very faint finite stars nonzero until the floor pass
}

/// Inputs already passed the full-footprint check for this viewport. Normal Rust bounds checks stay enabled.
pub(in crate::scene) fn draw_pixel_stars(layer: &mut [StarPixel], width: usize, zoom_boost: f64, stars: impl IntoIterator<Item = PixelStarKey>) -> usize {
    let mut submitted = 0;
    for star in stars {
        let rgb = star.color.map(|c| f32::from(c) / 255.0);
        let opacity = calculate_star_opacity(star.magnitude, zoom_boost); // apply the chosen opacity to each pixel without reducing it
        let (y, x) = (star.cell.0 as usize, star.cell.1 as usize);
        let bottom_right = y * width + x;
        for offset in [bottom_right - width - 1, bottom_right - width, bottom_right - 1, bottom_right] {
            blend_star_pixel(&mut layer[offset], rgb, opacity); // no repeated clipping checks inside the four-pixel loop
        }
        submitted += 1;
    }
    submitted
}

fn blend_star_pixel(pixel: &mut StarPixel, rgb: [f32; 3], opacity: f32) {
    if opacity == 0.0 { return; }
    let old_weight = pixel.opacity * (1.0 - opacity); // a more opaque new star leaves less of the previous color
    let combined_opacity = old_weight + opacity;
    for (old, new) in pixel.rgb.iter_mut().zip(rgb) {
        *old = (*old * old_weight + new * opacity) / combined_opacity;
    }
    pixel.opacity = combined_opacity;
}

pub(in crate::scene) fn apply_minimum_star_opacity(layer: &mut [StarPixel]) {
    for pixel in layer {
        if pixel.opacity > 0.0 { pixel.opacity = pixel.opacity.max(MIN_STAR_PIXEL_OPACITY); } // preserve empty pixels and unscaled RGB
    }
}

pub(in crate::scene) fn composite_star_layer(canvas: &mut Pixmap, layer: &[StarPixel]) {
    assert_eq!(canvas.data().len() / 4, layer.len()); // one star pixel per scene pixel, with matching row order
    for (target, star) in canvas.data_mut().chunks_exact_mut(4).zip(layer) {
        if star.opacity == 0.0 { continue; }
        for (channel, color) in target[..3].iter_mut().zip(star.rgb) {
            *channel = (color * 255.0 * star.opacity + f32::from(*channel) * (1.0 - star.opacity)).round() as u8;
        }
        target[3] = 255; // the scene background is opaque; tiny-skia can safely draw other objects on it
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgb: [f32; 3], opacity: f32) -> StarPixel { StarPixel { rgb: rgb.map(|c| c / 255.0), opacity } }
    fn close(actual: f32, expected: f32) { assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}"); }

    #[test]
    fn source_over_color_weights_attenuate_the_existing_orange_pixel() {
        let mut actual = pixel([255.0, 231.0, 176.0], 0.05);
        blend_star_pixel(&mut actual, [176.0 / 255.0, 229.0 / 255.0, 1.0], 0.1);
        for (a, e) in actual.rgb.into_iter().zip([5815.0 / 7395.0, 6659.0 / 7395.0, 6684.0 / 7395.0]) { close(a, e); } // old/new color weights are 9/29 and 20/29
        close(actual.opacity, 0.145);
        let original = actual;
        blend_star_pixel(&mut actual, [0.0; 3], 0.0);
        assert_eq!(actual, original);
    }

    #[test]
    fn empty_same_color_and_three_star_sequences_preserve_source_over() {
        let mut actual = StarPixel::default();
        blend_star_pixel(&mut actual, [1.0, 0.0, 0.0], 0.1);
        assert_eq!(actual, pixel([255.0, 0.0, 0.0], 0.1));
        blend_star_pixel(&mut actual, [1.0, 0.0, 0.0], 0.1);
        close(actual.opacity, 0.19);
        assert_eq!(actual.rgb, [1.0, 0.0, 0.0]);
        blend_star_pixel(&mut actual, [0.0, 0.0, 1.0], 0.2);
        close(actual.rgb[0], 0.152 / 0.352);
        close(actual.rgb[2], 0.2 / 0.352);
        close(actual.opacity, 0.352);
        let mut reversed = StarPixel::default();
        for (rgb, alpha) in [([0.0, 0.0, 1.0], 0.2), ([1.0, 0.0, 0.0], 0.1), ([1.0, 0.0, 0.0], 0.1)] {
            blend_star_pixel(&mut reversed, rgb, alpha);
        }
        close(reversed.opacity, actual.opacity);
        assert_ne!(reversed.rgb, actual.rgb); // later stars cover earlier colors, so drawing order still matters
    }

    #[test]
    fn new_star_opacity_controls_its_color_share_over_an_opaque_pixel() {
        for opacity in [0.2, 0.5, 0.8, 1.0] {
            let mut actual = pixel([255.0, 0.0, 0.0], 1.0);
            blend_star_pixel(&mut actual, [0.0, 0.0, 1.0], opacity);
            close(actual.rgb[0], 1.0 - opacity);
            close(actual.rgb[2], opacity);
            assert_eq!(actual.opacity, 1.0);
        }
        let mut actual = pixel([255.0; 3], 1.0);
        let orange = [1.0, 231.0 / 255.0, 176.0 / 255.0];
        blend_star_pixel(&mut actual, orange, 1.0);
        assert_eq!(actual.rgb, orange); // a fully opaque new star replaces the old color completely
    }

    #[test]
    fn opacity_floor_preserves_rgb_and_zero_pixels_including_very_faint_stars() {
        let mut layer = vec![StarPixel::default(), pixel([255.0, 231.0, 176.0], 1e-24),
            pixel([30.0, 40.0, 50.0], MIN_STAR_PIXEL_OPACITY), pixel([10.0, 20.0, 30.0], 1.0)];
        let before = layer.clone();
        apply_minimum_star_opacity(&mut layer);
        assert_eq!(layer[0], StarPixel::default());
        assert_eq!(layer[1].opacity, MIN_STAR_PIXEL_OPACITY);
        assert_eq!(layer[2..], before[2..]);
        assert_eq!(layer.iter().map(|p| p.rgb).collect::<Vec<_>>(), before.iter().map(|p| p.rgb).collect::<Vec<_>>());
        close(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE, 1.0), 1.0);
        close(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE + 2.0 / STAR_OPACITY_MAGNITUDE_SCALE, 1.0), 0.01);
        assert_eq!(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE - 1.0, 1.0), 1.0);
        assert!(calculate_star_opacity(55.535, 1.0) > 0.0);
    }

    #[test]
    fn zoom_boost_preserves_wide_views_and_brightens_narrow_views_continuously() {
        let reference = STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES;
        assert_eq!(calculate_zoom_opacity_boost(reference), 1.0);
        assert_eq!(calculate_zoom_opacity_boost(reference * 2.0), 1.0);
        let half = calculate_zoom_opacity_boost(reference / 2.0);
        assert!((half - 2_f64.powf(STAR_BRIGHTNESS_ZOOM_POWER)).abs() < 1e-12);
        let quarter = calculate_zoom_opacity_boost(reference / 4.0);
        assert!((quarter - half * half).abs() < 1e-12);
        assert!((calculate_zoom_opacity_boost(reference - 1e-6) - 1.0).abs() < 1e-6);
        assert!(calculate_zoom_opacity_boost(MIN_FOV_DEGREES).is_finite());

        let magnitude = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;
        close(calculate_star_opacity(magnitude, 1.0), 0.1);
        close(calculate_star_opacity(magnitude, half), (0.1 * half).min(1.0) as f32);
        assert_eq!(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE, quarter), 1.0);
    }

    #[test]
    fn composition_applies_opacity_once_over_black_and_colored_backgrounds() {
        let layer = [pixel([255.0, 231.0, 176.0], 1.0), pixel([255.0, 231.0, 176.0], 0.2), StarPixel::default()];
        let mut black = Pixmap::new(3, 1).unwrap();
        black.fill(tiny_skia::Color::BLACK);
        composite_star_layer(&mut black, &layer);
        assert_eq!(black.data(), &[255, 231, 176, 255, 51, 46, 35, 255, 0, 0, 0, 255]);
        let mut colored = Pixmap::new(3, 1).unwrap();
        colored.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        composite_star_layer(&mut colored, &layer);
        assert_eq!(&colored.data()[4..], &[59, 62, 59, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn all_four_pixels_receive_full_star_opacity_at_small_and_large_dimensions() {
        for (width, height, x, y) in [(2, 2, 1, 1), (4097, 3, 4096, 2), (3, 4097, 2, 4096)] {
            let viewport = ProjectionViewport { width, height };
            let mut layer = Vec::new();
            initialize_star_layer(&mut layer, viewport).unwrap();
            assert!(pixel_star_fits((y, x), viewport));
            let star = PixelStarKey { cell: (y, x), magnitude: STAR_OPACITY_REFERENCE_MAGNITUDE, color: [255, 0, 0] };
            assert_eq!(draw_pixel_stars(&mut layer, width, 1.0, [star]), 1);
            let lit: Vec<_> = layer.iter().filter(|p| p.opacity > 0.0).collect();
            assert_eq!(lit.len(), 4);
            assert!(lit.iter().all(|p| p.opacity == 1.0 && p.rgb == [1.0, 0.0, 0.0]));

            initialize_star_layer(&mut layer, viewport).unwrap();
            draw_pixel_stars(&mut layer, width, 1.0, [PixelStarKey { magnitude: STAR_OPACITY_REFERENCE_MAGNITUDE + 2.0 / STAR_OPACITY_MAGNITUDE_SCALE, ..star }]);
            let lit: Vec<_> = layer.iter().filter(|p| p.opacity > 0.0).collect();
            assert_eq!(lit.len(), 4);
            for pixel in lit { close(pixel.opacity, 0.01); } // faint stars retain the configured curve too
        }
    }

    #[test]
    fn footprint_checks_tiny_edges_and_layer_reuses_capacity_without_old_pixels() {
        let viewport = ProjectionViewport { width: 4, height: 3 };
        for cell in [(0, 1), (1, 0), (-1, 1), (1, -1), (3, 1), (1, 4), (i32::MAX, i32::MAX)] { assert!(!pixel_star_fits(cell, viewport)); }
        assert!(pixel_star_fits((2, 3), viewport));
        assert!(!pixel_star_fits((0, 0), ProjectionViewport { width: 1, height: 1 }));
        let mut layer = Vec::new();
        initialize_star_layer(&mut layer, viewport).unwrap();
        let allocation = layer.as_ptr();
        layer[0] = pixel([255.0; 3], 1.0);
        initialize_star_layer(&mut layer, viewport).unwrap();
        assert_eq!(layer.as_ptr(), allocation);
        assert!(layer.iter().all(|p| *p == StarPixel::default()));
        initialize_star_layer(&mut layer, ProjectionViewport { width: 2, height: 2 }).unwrap();
        assert_eq!(layer.as_ptr(), allocation);
        assert!(initialize_star_layer(&mut layer, ProjectionViewport { width: 0, height: 3 }).is_none());
        assert!(initialize_star_layer(&mut layer, ProjectionViewport { width: usize::MAX, height: 2 }).is_none());
    }
}
