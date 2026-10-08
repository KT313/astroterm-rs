//! Fixed four-pixel stars with straight RGB and independent opacity. No drawing-library blending is used here.
use crate::constants::{DYNAMIC_NAME_COUNT, MAX_IMAGE_PIXELS, MIN_STAR_PIXEL_OPACITY,
    STAR_OPACITY_REFERENCE_MAGNITUDE, STAR_OPACITY_MAGNITUDE_SCALE};
use crate::model::{Cell, PixelStarKey, ProjectedSky, ProjectionViewport, RenderOptions, StarPixel};
use tiny_skia::Pixmap;

const PIXEL_COVERAGE: f32 = 0.25; // each of the four neighboring pixels receives a quarter-strength contribution

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

fn calculate_star_opacity(magnitude: f64) -> f32 {
    let exponent = -STAR_OPACITY_MAGNITUDE_SCALE * (magnitude - STAR_OPACITY_REFERENCE_MAGNITUDE);
    10_f64.powf(exponent).clamp(f64::from(f32::MIN_POSITIVE), 1.0) as f32 // keep very faint finite stars nonzero until the floor pass
}

/// Inputs already passed the full-footprint check for this viewport. Normal Rust bounds checks stay enabled.
pub(in crate::scene) fn draw_pixel_stars(layer: &mut [StarPixel], width: usize, stars: impl IntoIterator<Item = PixelStarKey>) -> usize {
    let mut submitted = 0;
    for star in stars {
        let rgb = star.color.map(|c| f32::from(c) / 255.0);
        let opacity = calculate_star_opacity(star.magnitude) * PIXEL_COVERAGE;
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
    let weight_sum = pixel.opacity + opacity;
    for (old, new) in pixel.rgb.iter_mut().zip(rgb) {
        *old = (*old * pixel.opacity + new * opacity) / weight_sum;
    }
    pixel.opacity = opacity + pixel.opacity * (1.0 - opacity);
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
    fn custom_color_weights_and_opacity_match_the_orange_blue_example() {
        let mut actual = pixel([255.0, 231.0, 176.0], 0.05);
        blend_star_pixel(&mut actual, [176.0 / 255.0, 229.0 / 255.0, 1.0], 0.1);
        for (a, e) in actual.rgb.into_iter().zip([607.0 / 765.0, 689.0 / 765.0, 686.0 / 765.0]) { close(a, e); }
        close(actual.opacity, 0.145);
        let original = actual;
        blend_star_pixel(&mut actual, [0.0; 3], 0.0);
        assert_eq!(actual, original);
    }

    #[test]
    fn empty_same_color_and_three_star_sequences_preserve_the_custom_rule() {
        let mut actual = StarPixel::default();
        blend_star_pixel(&mut actual, [1.0, 0.0, 0.0], 0.1);
        assert_eq!(actual, pixel([255.0, 0.0, 0.0], 0.1));
        blend_star_pixel(&mut actual, [1.0, 0.0, 0.0], 0.1);
        close(actual.opacity, 0.19);
        assert_eq!(actual.rgb, [1.0, 0.0, 0.0]);
        blend_star_pixel(&mut actual, [0.0, 0.0, 1.0], 0.2);
        close(actual.rgb[0], 0.19 / 0.39);
        close(actual.rgb[2], 0.2 / 0.39);
        close(actual.opacity, 0.352);
        let mut reversed = StarPixel::default();
        for (rgb, alpha) in [([0.0, 0.0, 1.0], 0.2), ([1.0, 0.0, 0.0], 0.1), ([1.0, 0.0, 0.0], 0.1)] {
            blend_star_pixel(&mut reversed, rgb, alpha);
        }
        close(reversed.opacity, actual.opacity);
        assert_ne!(reversed.rgb, actual.rgb); // combined opacity is not the original sum of color weights
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
        close(calculate_star_opacity(0.0), 1.0);
        close(calculate_star_opacity(5.0), 0.01);
        assert_eq!(calculate_star_opacity(-10.0), 1.0);
        assert!(calculate_star_opacity(55.535) * PIXEL_COVERAGE > 0.0);
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
    fn four_pixels_have_equal_coverage_with_no_large_canvas_fallback() {
        for (width, height, x, y) in [(2, 2, 1, 1), (4097, 3, 4096, 2), (3, 4097, 2, 4096)] {
            let viewport = ProjectionViewport { width, height };
            let mut layer = Vec::new();
            initialize_star_layer(&mut layer, viewport).unwrap();
            assert!(pixel_star_fits((y, x), viewport));
            let star = PixelStarKey { cell: (y, x), magnitude: 0.0, color: [255, 0, 0] };
            assert_eq!(draw_pixel_stars(&mut layer, width, [star]), 1);
            let lit: Vec<_> = layer.iter().filter(|p| p.opacity > 0.0).collect();
            assert_eq!(lit.len(), 4);
            assert!(lit.iter().all(|p| p.opacity == 0.25 && p.rgb == [1.0, 0.0, 0.0]));
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
