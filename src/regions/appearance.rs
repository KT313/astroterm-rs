//! How a star looks in a tile: the fixed four-texel glyph, the opacity curve with its zoom boost, and the
//! premultiplied screen blend. These are the pixel renderer's rules (`scene/raster/pixels/stars.rs` reads
//! the curve from here until chunk 14 removes it), so a tile shows a star exactly as the screen did.
use crate::catalog::decode_magnitude;
use crate::constants::{MIN_FOV_DEGREES, STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES, STAR_BRIGHTNESS_ZOOM_POWER,
    STAR_OPACITY_MAGNITUDE_SCALE, STAR_OPACITY_REFERENCE_MAGNITUDE};
use crate::model::{StarColor, Texel, StarOpacities};

/// Whether a star of current magnitude `magnitude_code` is drawn at `threshold`: bright enough, and placed by the
/// simulation. Drawn stars need no particular order: the screen blend gives the same result for any order, up to
/// f32 rounding, so callers draw them in table order.
pub fn is_star_drawn(magnitude_code: u16, threshold: u16) -> bool { magnitude_code <= threshold && magnitude_code != crate::regions::UNPLACED }

/// How far the glyph reaches from the star's rounded position, in texels: the four texels around the rounded
/// position (the cell and the three above and to the left), plus the half texel of rounding.
pub const STAR_GLYPH_RADIUS: usize = 2;

impl Texel {
    pub const SIZE: usize = std::mem::size_of::<Texel>();
    pub fn is_empty(self) -> bool { self.opacity == 0.0 }
    #[cfg(feature = "pixel-debug")]
    #[track_caller]
    pub(crate) fn check(self, operation: &str, maximum: f64) {
        crate::pixel_debug::check_pixel(operation, [self.rgb[0] as f64, self.rgb[1] as f64, self.rgb[2] as f64, self.opacity as f64], maximum);
    }
    pub fn plus(self, other: Texel) -> Texel {
        #[cfg(feature = "pixel-debug")]
        { self.check("Texel::plus left", f64::INFINITY); other.check("Texel::plus right", f64::INFINITY); }
        let result = Texel { rgb: [self.rgb[0] + other.rgb[0], self.rgb[1] + other.rgb[1], self.rgb[2] + other.rgb[2]], opacity: self.opacity + other.opacity };
        #[cfg(feature = "pixel-debug")]
        result.check("Texel::plus output", f64::INFINITY);
        result
    }
    pub fn scaled(self, factor: f32) -> Texel {
        #[cfg(feature = "pixel-debug")]
        {
            self.check("Texel::scaled input", f64::INFINITY);
            assert!(factor.is_finite() && factor >= 0.0, "pixel-debug: Texel::scaled invalid factor={factor:?}");
        }
        let result = Texel { rgb: self.rgb.map(|c| c * factor), opacity: self.opacity * factor };
        #[cfg(feature = "pixel-debug")]
        result.check("Texel::scaled output", f64::INFINITY);
        result
    }
}

/// A visual compensation for fewer overlapping stars when zoomed in, independent of catalog brightness: 1 at the
/// reference field of view or wider, growing as the field narrows.
pub fn zoom_opacity_boost(fov_degrees: f64) -> f64 {
    let zoom = (STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES / fov_degrees.max(MIN_FOV_DEGREES)).max(1.0);
    zoom.powf(STAR_BRIGHTNESS_ZOOM_POWER)
}

/// The opacity curve: full at the reference magnitude, falling by a fixed factor per magnitude, boosted for
/// narrow views, and never zero so that the final frame can still raise a faint star to the visible floor.
pub fn star_opacity(magnitude: f64, zoom_boost: f64) -> f32 {
    let exponent = -STAR_OPACITY_MAGNITUDE_SCALE * (magnitude - STAR_OPACITY_REFERENCE_MAGNITUDE);
    (10_f64.powf(exponent) * zoom_boost).clamp(f64::from(f32::MIN_POSITIVE), 1.0) as f32
}

/// The opacity of every catalog magnitude code for one zoom boost, so that a star costs one table read.
impl StarOpacities {
    pub fn zoom_boost(&self) -> f64 { self.zoom_boost }

    /// Hold the curve for `zoom_boost`, rebuilding the table only when the boost changed. Returns whether it did.
    pub fn prepare(&mut self, zoom_boost: f64) -> bool {
        if self.zoom_boost == zoom_boost && self.opacities.len() == usize::from(u16::MAX) + 1 { return false; }
        self.opacities.clear();
        self.opacities.extend((0..=u16::MAX).map(|code| star_opacity(decode_magnitude(code), zoom_boost)));
        self.zoom_boost = zoom_boost;
        true
    }

    pub fn of(&self, magnitude_code: u16) -> f32 { self.opacities[usize::from(magnitude_code)] }
}

/// Exact `level / 255` for every 8-bit channel value, so stars need no division per colour.
const UNIT_LEVELS: [f32; 256] = {
    let mut levels = [0.0; 256];
    let mut level = 0;
    while level < 256 { levels[level] = level as f32 / 255.0; level += 1; }
    levels
};

/// The colour share a star of palette colour `color` adds at `opacity`.
pub fn premultiplied_color(color: u8, opacity: f32) -> [f32; 3] {
    #[cfg(feature = "pixel-debug")]
    crate::pixel_debug::check_channel("premultiplied_color input", 3, opacity as f64, 1.0);
    let rgb = StarColor::rgb_of(color).map(|level| UNIT_LEVELS[usize::from(level)] * opacity);
    #[cfg(feature = "pixel-debug")]
    (Texel { rgb, opacity }).check("premultiplied_color output", 1.0);
    rgb
}

/// Screen blending in premultiplied form, the rule regions are composed with on the screen: every channel
/// becomes `old + new · (1 − old)`, so overlapping stars brighten each other, neither covers the other, and the
/// result does not depend on their order (up to f32 rounding). Opacity combines as `1 − (1 − old)(1 − opacity)`.
/// Channels stay within [0, 1]; the cap only absorbs last-bit rounding. Zero opacity changes nothing.
#[inline]
pub fn blend_star(texel: &mut Texel, premultiplied: [f32; 3], opacity: f32) {
    #[cfg(feature = "pixel-debug")]
    {
        texel.check("blend_star destination", 1.0);
        (Texel { rgb: premultiplied, opacity }).check("blend_star source", 1.0);
    }
    for (old, new) in texel.rgb.iter_mut().zip(premultiplied) { *old = (*old + new * (1.0 - *old)).min(1.0); }
    texel.opacity = (texel.opacity + opacity * (1.0 - texel.opacity)).min(1.0);
    #[cfg(feature = "pixel-debug")]
    texel.check("blend_star output", 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::magnitude_code;

    fn close(actual: f32, expected: f32) { assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}"); }

    #[test]
    fn the_curve_is_full_at_the_reference_magnitude_and_falls_by_the_configured_factor() {
        close(star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE, 1.0), 1.0);
        close(star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE + 2.0 / STAR_OPACITY_MAGNITUDE_SCALE, 1.0), 0.01);
        assert_eq!(star_opacity(STAR_OPACITY_REFERENCE_MAGNITUDE - 1.0, 1.0), 1.0);
        assert!(star_opacity(55.535, 1.0) > 0.0);                                             // never zero: the floor can still lift it
    }

    #[test]
    fn the_zoom_boost_keeps_wide_views_and_brightens_narrow_ones_continuously() {
        let reference = STAR_BRIGHTNESS_REFERENCE_FOV_DEGREES;
        assert_eq!(zoom_opacity_boost(reference), 1.0);
        assert_eq!(zoom_opacity_boost(reference * 2.0), 1.0);
        let half = zoom_opacity_boost(reference / 2.0);
        assert!((half - 2_f64.powf(STAR_BRIGHTNESS_ZOOM_POWER)).abs() < 1e-12);
        assert!((zoom_opacity_boost(reference / 4.0) - half * half).abs() < 1e-12);
        assert!(zoom_opacity_boost(MIN_FOV_DEGREES).is_finite());
        let magnitude = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;
        close(star_opacity(magnitude, 1.0), 0.1);
        close(star_opacity(magnitude, half), (0.1 * half).min(1.0) as f32);
    }

    #[test]
    fn the_table_follows_the_curve_per_code_and_is_rebuilt_only_for_a_new_boost() {
        let mut table = StarOpacities::default();
        assert!(table.prepare(1.0));
        assert!(!table.prepare(1.0));
        for code in (0..=u16::MAX).step_by(997).chain([u16::MAX]) { assert_eq!(table.of(code), star_opacity(decode_magnitude(code), 1.0)); }
        let allocation = table.opacities.as_ptr();
        assert!(table.prepare(2.0));
        assert_eq!(table.opacities.as_ptr(), allocation);                                   // a new boost refills the same allocation
        assert_eq!(table.of(magnitude_code(2.0)), star_opacity(2.0, 2.0));
        let dim = magnitude_code(STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE);
        assert!(table.of(dim) > star_opacity(decode_magnitude(dim), 1.0));                   // choose an unsaturated magnitude even when the constants are tuned
    }

    #[test]
    fn blending_is_screen_blending_and_adding_is_commutative() {
        let mut texel = Texel::default();
        blend_star(&mut texel, premultiplied_color(StarColor::RedOrange.index(), 1.0), 1.0);
        assert_eq!(texel, Texel { rgb: StarColor::RedOrange.rgb().map(|level| level as f32 / 255.0), opacity: 1.0 });
        let mut faint = Texel::default();
        blend_star(&mut faint, premultiplied_color(0, 0.1), 0.1);
        blend_star(&mut faint, premultiplied_color(0, 0.1), 0.1);
        close(faint.opacity, 0.19);                                                          // 0.1 + 0.9 × 0.1
        blend_star(&mut faint, [0.0; 3], 0.0);
        close(faint.opacity, 0.19);

        // Half-opaque red and blue: each keeps its full colour whichever is drawn first.
        let (red, blue) = ([0.5, 0.0, 0.0], [0.0, 0.0, 0.5]);
        let (mut red_first, mut blue_first) = (Texel::default(), Texel::default());
        blend_star(&mut red_first, red, 0.5);
        blend_star(&mut red_first, blue, 0.5);
        blend_star(&mut blue_first, blue, 0.5);
        blend_star(&mut blue_first, red, 0.5);
        assert_eq!(red_first, blue_first);
        assert_eq!(red_first, Texel { rgb: [0.5, 0.0, 0.5], opacity: 0.75 });
        assert_eq!(texel.plus(faint), faint.plus(texel));
        assert_eq!(Texel::SIZE, 16);
        assert!(Texel::default().is_empty() && !texel.is_empty());
    }

    #[cfg(feature = "pixel-debug")]
    #[test]
    fn pixel_wrappers_detect_bad_blend_opacity_and_float_overflow_at_the_operation() {
        let failure = std::panic::catch_unwind(|| blend_star(&mut Texel::default(), [0.0; 3], 1.1)).unwrap_err();
        assert!(failure.downcast_ref::<String>().unwrap().contains("blend_star source: opacity"));
        let huge = Texel { rgb: [f32::MAX; 3], opacity: f32::MAX };
        let failure = std::panic::catch_unwind(|| huge.plus(huge)).unwrap_err();
        assert!(failure.downcast_ref::<String>().unwrap().contains("Texel::plus output"));
        assert!(std::panic::catch_unwind(|| huge.scaled(2.0)).is_err());
        assert!(std::panic::catch_unwind(|| Texel::default().scaled(-1.0)).is_err());
    }
}
