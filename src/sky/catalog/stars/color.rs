//! Classify source color inputs once, using the original f32 thresholds and spectral-letter precedence.
use crate::model::StarColor;

pub(super) fn classify_star_color(spectral: [u8; 2], bv: Option<f32>) -> StarColor {
    match spectral[0] {
        b'O' | b'W' => StarColor::HotBlue,
        b'B' => StarColor::BlueWhite,
        b'A' => StarColor::White,
        b'F' => StarColor::YellowWhite,
        b'G' => StarColor::Yellow,
        b'K' => StarColor::Orange,
        b'M' | b'C' | b'S' | b'N' => StarColor::RedOrange,
        _ => match bv {
            Some(value) if value < 0.0 => StarColor::BlueWhite,
            Some(value) if value >= 1.4 => StarColor::RedOrange,
            Some(value) if value >= 0.8 => StarColor::Orange,
            _ => StarColor::Default,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Color;

    // Independent copies of the pre-migration rendering rules guard both outputs, including unknown prefixes.
    fn original(spectral: u8, bv: Option<f32>) -> ([u8; 3], Option<Color>) {
        let rgb = match spectral {
            b'O' | b'W' => [155, 185, 255], b'B' => [180, 205, 255], b'A' => [220, 231, 255],
            b'F' => [248, 245, 235], b'G' => [255, 234, 192], b'K' => [255, 192, 125],
            b'M' | b'C' | b'S' | b'N' => [255, 142, 91],
            _ => match bv { Some(v) if v < 0.0 => [180, 205, 255], Some(v) if v >= 1.4 => [255, 142, 91],
                Some(v) if v >= 0.8 => [255, 192, 125], _ => [230, 236, 255] },
        };
        let terminal = match spectral {
            b'O' | b'B' | b'W' => Some(Color::Cyan), b'A' | b'F' | b'G' => None,
            b'K' => Some(Color::Yellow), b'M' | b'C' | b'S' | b'N' => Some(Color::Red),
            _ => match bv { Some(v) if v < 0.0 => Some(Color::Cyan), Some(v) if v >= 1.4 => Some(Color::Red),
                Some(v) if v >= 0.8 => Some(Color::Yellow), _ => None },
        };
        (rgb, terminal)
    }
    #[test]
    fn palette_matches_both_original_renderers_for_every_prefix_and_boundary() {
        let values = [None, Some(-2.401), Some(0.0_f32.next_down()), Some(-0.0), Some(0.0), Some(0.0_f32.next_up()),
            Some(0.8_f32.next_down()), Some(0.8), Some(0.8_f32.next_up()), Some(1.4_f32.next_down()), Some(1.4),
            Some(1.4_f32.next_up()), Some(6.581)];
        for first in 0..=u8::MAX {
            for second in 0..=u8::MAX {
                for value in values {
                    let color = classify_star_color([first, second], value);
                    assert_eq!((color.rgb(), color.terminal_color()), original(first, value));
                    assert_eq!(StarColor::from_index(color.index()), Some(color));
                }
            }
        }
        for invalid in 8..=u8::MAX { assert_eq!(StarColor::from_index(invalid), None); }
        assert_eq!(std::mem::size_of::<StarColor>(), 1);
    }
}
