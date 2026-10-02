//! How objects look on the character grid: a glyph for each character set, an optional label and color.

use crate::astro::{MoonPhase, map_float_to_int_range};
use crate::canvas::Color;
use crate::sky::{PlanetKind, Star};

/// Brightest and dimmest magnitudes in the star catalog, used to pick star glyphs.
const BRIGHTEST_STAR_MAGNITUDE: f64 = -1.46;
const DIMMEST_STAR_MAGNITUDE: f64 = 7.96;

/// Star glyphs from brightest to dimmest.
const STAR_GLYPHS_UNICODE: [char; 10] = ['⬤', '●', '⦁', '•', '•', '∙', '⋅', '⋅', '⋅', '⋅'];
const STAR_GLYPHS_ASCII: [char; 10] = ['0', '0', 'O', 'O', 'o', 'o', '.', '.', '.', '.'];

/// How an object is drawn: a glyph for each character set, an optional label next to it, and an optional color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub ascii: char,
    pub unicode: char,
    pub label: Option<&'static str>,
    pub color: Option<Color>,
}

/// A star's look: a bigger glyph the brighter it is, its name, and a color from its spectral class.
pub fn select_star_appearance(star: &Star) -> Appearance {
    let glyph_index = select_star_glyph_index(star.magnitude);
    Appearance {
        ascii: STAR_GLYPHS_ASCII[glyph_index],
        unicode: STAR_GLYPHS_UNICODE[glyph_index],
        label: star.name,
        color: select_star_color(star.spectral_type),
    }
}

/// The look of the Sun or a planet: its astronomical symbol and name.
pub fn select_planet_appearance(kind: PlanetKind) -> Appearance {
    let (ascii, unicode, color) = match kind {
        PlanetKind::Sun => ('@', '☉', Color::Yellow),
        PlanetKind::Mercury => ('*', '☿', Color::White),
        PlanetKind::Venus => ('*', '♀', Color::Yellow),
        PlanetKind::Mars => ('*', '♂', Color::Red),
        PlanetKind::Jupiter => ('*', '♃', Color::Magenta),
        PlanetKind::Saturn => ('*', '♄', Color::Yellow),
        PlanetKind::Uranus => ('*', '⛢', Color::Cyan),
        PlanetKind::Neptune => ('*', '♆', Color::Blue),
    };
    Appearance {
        ascii,
        unicode,
        label: Some(kind.name()),
        color: Some(color),
    }
}

/// The Moon's look: an emoji of its phase, lit on the right or the left side as seen on screen.
pub fn select_moon_appearance(phase: MoonPhase, lit_on_right: bool) -> Appearance {
    let (right, left) = match phase {
        MoonPhase::New => ('🌑', '🌑'),
        MoonPhase::Full => ('🌕', '🌕'),
        MoonPhase::WaxingCrescent | MoonPhase::WaningCrescent => ('🌒', '🌘'),
        MoonPhase::FirstQuarter | MoonPhase::LastQuarter => ('🌓', '🌗'),
        MoonPhase::WaxingGibbous | MoonPhase::WaningGibbous => ('🌔', '🌖'),
    };
    Appearance {
        ascii: 'M',
        unicode: if lit_on_right { right } else { left },
        label: Some("Moon"),
        color: None,
    }
}

/// Approximate color of a star from its spectral class, within the 8 basic terminal colors: hot blue-white stars
/// (O, B, Wolf-Rayet) are cyan, white to yellow-white stars (A, F, G) use the default color, orange K stars are yellow
/// and cool red giants and carbon stars (M, C, S, N) are red.
fn select_star_color(spectral_type: [u8; 2]) -> Option<Color> {
    match spectral_type[0] {
        b'O' | b'B' | b'W' => Some(Color::Cyan),
        b'K' => Some(Color::Yellow),
        b'M' | b'C' | b'S' | b'N' => Some(Color::Red),
        _ => None,
    }
}

/// Index into the star glyph tables for a magnitude (brighter stars get bigger glyphs).
fn select_star_glyph_index(magnitude: f32) -> usize {
    let last = STAR_GLYPHS_ASCII.len() as i32 - 1;
    let index = map_float_to_int_range(
        BRIGHTEST_STAR_MAGNITUDE,
        DIMMEST_STAR_MAGNITUDE,
        0,
        last,
        f64::from(magnitude),
    );
    index.clamp(0, last) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::load_embedded_catalog;
    use crate::sky::Sky;

    #[test]
    fn star_colors_follow_spectral_class() {
        assert_eq!(select_star_color(*b"B8"), Some(Color::Cyan)); // Rigel
        assert_eq!(select_star_color(*b"WN"), Some(Color::Cyan));
        assert_eq!(select_star_color(*b"A0"), None); // Vega
        assert_eq!(select_star_color(*b"G2"), None); // like the Sun
        assert_eq!(select_star_color(*b"K1"), Some(Color::Yellow)); // Arcturus
        assert_eq!(select_star_color(*b"M1"), Some(Color::Red)); // Betelgeuse
        assert_eq!(select_star_color(*b"  "), None);
    }

    #[test]
    fn star_glyphs_scale_with_brightness() {
        assert_eq!(select_star_glyph_index(-1.46), 0);
        assert_eq!(select_star_glyph_index(7.96), 9);
        assert_eq!(select_star_glyph_index(-30.0), 0); // clamped
        assert_eq!(select_star_glyph_index(30.0), 9);
    }

    #[test]
    fn bright_stars_get_their_names_and_spectral_colors() {
        let sky = Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"));
        let star = |catalog_number: usize| select_star_appearance(&sky.stars[catalog_number - 1]);
        assert_eq!(
            (star(2061).label, star(2061).color),
            (Some("Betelgeuse"), Some(Color::Red))
        );
        assert_eq!((star(1713).label, star(1713).color), (Some("Rigel"), Some(Color::Cyan)));
        assert_eq!(
            (star(5340).label, star(5340).color),
            (Some("Arcturus"), Some(Color::Yellow))
        );
        assert_eq!((star(7001).label, star(7001).color), (Some("Vega"), None));
    }

    #[test]
    fn planets_are_labelled_with_their_names() {
        let jupiter = select_planet_appearance(PlanetKind::Jupiter);
        assert_eq!((jupiter.unicode, jupiter.label), ('♃', Some("Jupiter")));
        assert_eq!(select_planet_appearance(PlanetKind::Sun).ascii, '@');
    }

    #[test]
    fn moon_glyphs_show_the_phase_and_lit_side() {
        let glyphs = |lit_on_right| -> String {
            MoonPhase::ALL
                .iter()
                .map(|&phase| select_moon_appearance(phase, lit_on_right).unicode)
                .collect()
        };
        assert_eq!(glyphs(true), "🌑🌒🌓🌔🌕🌔🌓🌒");
        assert_eq!(glyphs(false), "🌑🌘🌗🌖🌕🌖🌗🌘");
    }
}
