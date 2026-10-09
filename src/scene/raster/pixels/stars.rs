//! Fixed four-pixel stars with premultiplied RGB and a tabulated opacity curve. No drawing-library blending is used here.
use crate::catalog::MIN_MAGNITUDE;
use crate::constants::{MAX_IMAGE_PIXELS, MIN_STAR_PIXEL_OPACITY,
    STAR_OPACITY_REFERENCE_MAGNITUDE, STAR_OPACITY_MAGNITUDE_SCALE, MIN_FOV_DEGREES,
    STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES, STAR_BRIGHTNESS_ZOOM_POWER};
use crate::model::{Cell, PixelStarKey, ProjectedSky, ProjectionViewport, RenderOptions, StarOpacityTable, StarPixel};
use tiny_skia::Pixmap;

/// One opacity per catalog magnitude code: thousandths of a magnitude from -10.000 through 55.535.
const OPACITY_TABLE_LEN: usize = u16::MAX as usize + 1;

/// Exact `level / 255` for every 8-bit channel value, so stars need no division per color.
const UNIT_LEVELS: [f32; 256] = {
    let mut levels = [0.0; 256];
    let mut level = 0;
    while level < 256 { levels[level] = level as f32 / 255.0; level += 1; }
    levels
};

/// Whether the star's four pixels (its cell and the three above and to the left) lie inside the viewport.
#[inline]
pub(crate) fn pixel_star_fits((y, x): Cell, viewport: ProjectionViewport) -> bool {
    x > 0 && y > 0 && (x as usize) < viewport.width && (y as usize) < viewport.height
}

/// Keep global label selection independent of regional painting order; omit undrawable edge stars.
pub(crate) fn select_pixel_star_labels(options: &RenderOptions, sky: &ProjectedSky<'_>) -> crate::scene::StarLabels {
    crate::scene::select_star_labels(options, sky, |cell| pixel_star_fits(cell, sky.viewport))
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

/// The opacity curve itself; stars read it through the table, so this runs once per code, not once per star.
fn calculate_star_opacity(magnitude: f64, zoom_boost: f64) -> f32 {
    let exponent = -STAR_OPACITY_MAGNITUDE_SCALE * (magnitude - STAR_OPACITY_REFERENCE_MAGNITUDE);
    (10_f64.powf(exponent) * zoom_boost).clamp(f64::from(f32::MIN_POSITIVE), 1.0) as f32 // keep very faint finite stars nonzero so the floor still lights them
}

/// Build the opacity table for this view's zoom boost unless it already holds it. Returns whether it was rebuilt.
pub(in crate::scene) fn prepare_star_opacities(table: &mut StarOpacityTable, fov_degrees: f64) -> bool {
    let zoom_boost = calculate_zoom_opacity_boost(fov_degrees);
    if table.zoom_boost == zoom_boost && table.opacities.len() == OPACITY_TABLE_LEN { return false; }
    table.opacities.clear();
    table.opacities.extend((0..OPACITY_TABLE_LEN).map(|code| calculate_star_opacity(crate::catalog::decode_magnitude(code as u16), zoom_boost)));
    table.zoom_boost = zoom_boost;
    true
}

/// Nearest catalog code, like `encode_magnitude`; the cast clamps magnitudes below -10 and NaN to the brightest entry.
#[inline]
fn look_up_star_opacity(table: &[f32; OPACITY_TABLE_LEN], magnitude: f64) -> f32 {
    let code = ((magnitude - MIN_MAGNITUDE) * 1000.0 + 0.5) as usize;
    table[code.min(OPACITY_TABLE_LEN - 1)]
}

/// Inputs already passed the full-footprint check for this viewport. Normal Rust bounds checks stay enabled.
pub(in crate::scene) fn draw_pixel_stars(layer: &mut [StarPixel], width: usize, opacities: &StarOpacityTable, stars: impl IntoIterator<Item = PixelStarKey>) -> usize {
    let table: &[f32; OPACITY_TABLE_LEN] = opacities.opacities.as_slice().try_into().expect("prepared star opacity table");
    let mut submitted = 0;
    for star in stars {
        let opacity = look_up_star_opacity(table, star.magnitude);
        let premultiplied = star.color.map(|level| UNIT_LEVELS[usize::from(level)] * opacity); // the star's own color share, computed once per star
        let (y, x) = (star.cell.0 as usize, star.cell.1 as usize);
        let bottom_right = y * width + x;
        for offset in [bottom_right - width - 1, bottom_right - width, bottom_right - 1, bottom_right] {
            blend_star_pixel(&mut layer[offset], premultiplied, opacity); // no repeated clipping checks inside the four-pixel loop
        }
        submitted += 1;
    }
    submitted
}

/// Source-over in premultiplied form: four multiply-adds per pixel and no division. Zero opacity leaves the pixel unchanged.
#[inline]
fn blend_star_pixel(pixel: &mut StarPixel, premultiplied: [f32; 3], opacity: f32) {
    let keep = 1.0 - opacity; // a more opaque new star leaves less of the previous color
    for (old, new) in pixel.rgb.iter_mut().zip(premultiplied) { *old = *old * keep + new; }
    pixel.opacity = pixel.opacity * keep + opacity;
}

/// Composite over the opaque background. Nonempty pixels fainter than `MIN_STAR_PIXEL_OPACITY` are raised to it without changing their color.
pub(in crate::scene) fn composite_star_layer(canvas: &mut Pixmap, layer: &[StarPixel]) {
    assert_eq!(canvas.data().len() / 4, layer.len()); // one star pixel per scene pixel, with matching row order
    for (target, star) in canvas.data_mut().chunks_exact_mut(4).zip(layer) {
        if star.opacity == 0.0 { continue; }
        let floor = star.opacity.max(MIN_STAR_PIXEL_OPACITY);
        let scale = if star.opacity < MIN_STAR_PIXEL_OPACITY { 255.0 * floor / star.opacity } else { 255.0 }; // divide only for the rare raised pixel
        let background = 1.0 - floor;
        for (channel, color) in target[..3].iter_mut().zip(star.rgb) {
            *channel = (color * scale + f32::from(*channel) * background + 0.5) as u8; // results stay within 0..=255, so adding one half and truncating rounds
        }
        target[3] = 255; // the scene background is opaque; tiny-skia can safely draw other objects on it
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgb: [f32; 3], opacity: f32) -> StarPixel { StarPixel { rgb: rgb.map(|c| c / 255.0 * opacity), opacity } }
    fn straight(pixel: StarPixel) -> [f32; 3] { pixel.rgb.map(|c| c / pixel.opacity) }
    fn blend(pixel: &mut StarPixel, rgb: [f32; 3], opacity: f32) { blend_star_pixel(pixel, rgb.map(|c| c * opacity), opacity); }
    fn close(actual: f32, expected: f32) { assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}"); }
    fn table(fov_degrees: f64) -> StarOpacityTable { let mut table = StarOpacityTable::default(); prepare_star_opacities(&mut table, fov_degrees); table }

    #[test]
    fn source_over_color_weights_attenuate_the_existing_orange_pixel() {
        let mut actual = pixel([255.0, 231.0, 176.0], 0.05);
        blend(&mut actual, [176.0 / 255.0, 229.0 / 255.0, 1.0], 0.1);
        for (a, e) in straight(actual).into_iter().zip([5815.0 / 7395.0, 6659.0 / 7395.0, 6684.0 / 7395.0]) { close(a, e); } // old/new color weights are 9/29 and 20/29
        close(actual.opacity, 0.145);
        let original = actual;
        blend(&mut actual, [0.0; 3], 0.0);
        assert_eq!(actual, original);
    }

    #[test]
    fn empty_same_color_and_three_star_sequences_preserve_source_over() {
        let mut actual = StarPixel::default();
        blend(&mut actual, [1.0, 0.0, 0.0], 0.1);
        assert_eq!(actual, pixel([255.0, 0.0, 0.0], 0.1));
        blend(&mut actual, [1.0, 0.0, 0.0], 0.1);
        close(actual.opacity, 0.19);
        for (a, e) in straight(actual).into_iter().zip([1.0, 0.0, 0.0]) { close(a, e); }
        blend(&mut actual, [0.0, 0.0, 1.0], 0.2);
        close(straight(actual)[0], 0.152 / 0.352);
        close(straight(actual)[2], 0.2 / 0.352);
        close(actual.opacity, 0.352);
        let mut reversed = StarPixel::default();
        for (rgb, alpha) in [([0.0, 0.0, 1.0], 0.2), ([1.0, 0.0, 0.0], 0.1), ([1.0, 0.0, 0.0], 0.1)] {
            blend(&mut reversed, rgb, alpha);
        }
        close(reversed.opacity, actual.opacity);
        assert_ne!(reversed.rgb, actual.rgb); // later stars cover earlier colors, so drawing order still matters
    }

    #[test]
    fn new_star_opacity_controls_its_color_share_over_an_opaque_pixel() {
        for opacity in [0.2, 0.5, 0.8, 1.0] {
            let mut actual = pixel([255.0, 0.0, 0.0], 1.0);
            blend(&mut actual, [0.0, 0.0, 1.0], opacity);
            close(actual.rgb[0], 1.0 - opacity);
            close(actual.rgb[2], opacity);
            close(actual.opacity, 1.0);
        }
        let mut actual = pixel([255.0; 3], 1.0);
        let orange = [1.0, 231.0 / 255.0, 176.0 / 255.0];
        blend(&mut actual, orange, 1.0);
        assert_eq!(actual.rgb, orange); // a fully opaque new star replaces the old color completely
    }

    #[test]
    fn opacity_table_follows_the_curve_and_quantizes_magnitudes_to_catalog_codes() {
        close(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE, 1.0), 1.0);
        close(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE + 2.0 / STAR_OPACITY_MAGNITUDE_SCALE, 1.0), 0.01);
        assert_eq!(calculate_star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE - 1.0, 1.0), 1.0);
        assert!(calculate_star_opacity(55.535, 1.0) > 0.0);

        let mut built = StarOpacityTable::default();
        assert!(prepare_star_opacities(&mut built, STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES));
        assert!(!prepare_star_opacities(&mut built, STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES * 2.0)); // the same boost keeps the table
        assert_eq!(built.zoom_boost, 1.0);
        assert_eq!(built.opacities.len(), OPACITY_TABLE_LEN);
        for code in (0..OPACITY_TABLE_LEN).step_by(997).chain([OPACITY_TABLE_LEN - 1]) {
            assert_eq!(built.opacities[code], calculate_star_opacity(crate::catalog::decode_magnitude(code as u16), 1.0));
        }
        let entries: &[f32; OPACITY_TABLE_LEN] = built.opacities.as_slice().try_into().unwrap();
        assert_eq!(look_up_star_opacity(entries, 2.0), built.opacities[12000]);
        assert_eq!(look_up_star_opacity(entries, 2.0004), built.opacities[12000]);
        assert_eq!(look_up_star_opacity(entries, 2.0006), built.opacities[12001]);
        assert_eq!(look_up_star_opacity(entries, -12.0), built.opacities[0]);
        assert_eq!(look_up_star_opacity(entries, f64::NAN), built.opacities[0]);
        assert_eq!(look_up_star_opacity(entries, 60.0), built.opacities[OPACITY_TABLE_LEN - 1]);
        let exact = calculate_star_opacity(16.6667, 1.0);
        assert!((look_up_star_opacity(entries, 16.6667) - exact).abs() < exact * 2e-4); // half a thousandth of a magnitude at most

        let allocation = built.opacities.as_ptr();
        assert!(prepare_star_opacities(&mut built, STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES / 2.0));
        assert_eq!(built.opacities.as_ptr(), allocation); // a new boost refills the same allocation
        assert_eq!(built.opacities[12000], calculate_star_opacity(2.0, built.zoom_boost));
        assert!(built.opacities[12000] > calculate_star_opacity(2.0, 1.0));
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
    fn composition_raises_faint_pixels_to_the_floor_without_changing_their_color_or_empty_pixels() {
        let layer = [StarPixel::default(), pixel([255.0, 231.0, 176.0], 1e-24), pixel([255.0, 231.0, 176.0], MIN_STAR_PIXEL_OPACITY),
            pixel([255.0, 231.0, 176.0], MIN_STAR_PIXEL_OPACITY * 0.5), pixel([10.0, 20.0, 30.0], 1.0)];
        let mut canvas = Pixmap::new(5, 1).unwrap();
        canvas.fill(tiny_skia::Color::from_rgba8(100, 100, 100, 255));
        composite_star_layer(&mut canvas, &layer);
        let data = canvas.data();
        assert_eq!(&data[..4], &[100, 100, 100, 255]);                                  // empty pixels keep the background
        assert_eq!(&data[8..12], &[108, 107, 104, 255]);                                // 5% of orange over 95% grey
        assert_eq!(data[4..8], data[8..12]);                                            // a vanishing opacity is raised to the floor, color intact
        assert_eq!(data[12..16], data[8..12]);
        assert_eq!(&data[16..], &[10, 20, 30, 255]);
    }

    #[test]
    fn all_four_pixels_receive_full_star_opacity_at_small_and_large_dimensions() {
        let opacities = table(STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES);
        for (width, height, x, y) in [(2, 2, 1, 1), (4097, 3, 4096, 2), (3, 4097, 2, 4096)] {
            let viewport = ProjectionViewport { width, height };
            let mut layer = Vec::new();
            initialize_star_layer(&mut layer, viewport).unwrap();
            assert!(pixel_star_fits((y, x), viewport));
            let star = PixelStarKey { cell: (y, x), magnitude: STAR_OPACITY_REFERENCE_MAGNITUDE, color: [255, 0, 0] };
            assert_eq!(draw_pixel_stars(&mut layer, width, &opacities, [star]), 1);
            let lit: Vec<_> = layer.iter().filter(|p| p.opacity > 0.0).collect();
            assert_eq!(lit.len(), 4);
            assert!(lit.iter().all(|p| p.opacity == 1.0 && p.rgb == [1.0, 0.0, 0.0]));

            initialize_star_layer(&mut layer, viewport).unwrap();
            draw_pixel_stars(&mut layer, width, &opacities, [PixelStarKey { magnitude: STAR_OPACITY_REFERENCE_MAGNITUDE + 2.0 / STAR_OPACITY_MAGNITUDE_SCALE, ..star }]);
            let lit: Vec<_> = layer.iter().filter(|p| p.opacity > 0.0).collect();
            assert_eq!(lit.len(), 4);
            for pixel in lit { close(pixel.opacity, 0.01); close(pixel.rgb[0], 0.01); } // faint stars retain the configured curve too
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
