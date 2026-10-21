//! Order-independent screen composition of projected regions and worker outputs. Inputs are stored
//! premultiplied RGB and opacity in [0, 1]; this is a visual blend, not a physical light sum.
use crate::model::Texel;

/// Add each incoming channel without attenuating the existing channel. In real arithmetic this is
/// a + b - a*b, or 1 - (1-a)*(1-b): symmetric and associative. f32 regrouping may change last bits.
#[inline]
pub(super) fn blend_screen_pixel(destination: &mut Texel, source: Texel) {
    #[cfg(feature = "pixel-debug")]
    {
        destination.check("screen blend destination", 1.0);
        source.check("screen blend source", 1.0);
    }

    // RGB is already opacity-weighted. Multiplying by opacity again would darken faint contributions.
    for (old, incoming) in destination.rgb.iter_mut().zip(source.rgb) {
        *old = (*old + incoming * (1.0 - *old)).min(1.0);
    }
    destination.opacity = (destination.opacity + source.opacity * (1.0 - destination.opacity)).min(1.0);

    #[cfg(feature = "pixel-debug")]
    destination.check("screen blend output", 1.0);
}

pub(super) fn blend_screen_images(destination: &mut [Texel], source: &[Texel]) {
    assert_eq!(destination.len(), source.len(), "projection images must have matching dimensions");
    for (pixel, &incoming) in destination.iter_mut().zip(source) {
        if incoming.is_empty() { continue; }
        blend_screen_pixel(pixel, incoming);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faint_light_and_transparency_never_darken_stored_channels() {
        let bright = Texel { rgb: [0.8, 0.4, 0.2], opacity: 0.9 };
        let mut result = bright;
        blend_screen_pixel(&mut result, Texel::default());
        assert_eq!(result, bright);
        for _ in 0..1000 {
            let previous = result;
            blend_screen_pixel(&mut result, Texel { rgb: [0.00001, 0.0001, 0.001], opacity: 0.001 });
            for (value, old) in result.rgb.into_iter().chain([result.opacity]).zip(previous.rgb.into_iter().chain([previous.opacity])) {
                assert!(value >= old && value <= 1.0);
            }
        }
    }

    #[test]
    fn screen_example_and_grouping_match_in_either_order() {
        let red = Texel { rgb: [0.4, 0.0, 0.0], opacity: 0.4 };
        let blue = Texel { rgb: [0.0, 0.0, 0.5], opacity: 0.5 };
        let green = Texel { rgb: [0.0, 0.3, 0.0], opacity: 0.3 };
        let mut forward = red;
        blend_screen_pixel(&mut forward, blue);
        assert_eq!(forward.rgb, [0.4, 0.0, 0.5]);
        assert!((forward.opacity - 0.7).abs() < 1e-6);
        blend_screen_pixel(&mut forward, green);
        for order in [[green, blue, red], [blue, red, green], [red, green, blue]] {
            let mut grouped = order[0];
            blend_screen_pixel(&mut grouped, order[1]);
            let mut result = order[2];
            blend_screen_pixel(&mut result, grouped);
            for (actual, expected) in result.rgb.into_iter().chain([result.opacity]).zip(forward.rgb.into_iter().chain([forward.opacity])) {
                assert!((actual - expected).abs() < 1e-6);
            }
        }
    }
}
