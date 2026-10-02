//! Celestial objects and how they look on screen.

use crate::astro::{Equatorial, Horizontal, MoonOrbit, MoonPhase, PlanetOrbit};
use crate::canvas::Color;
use crate::catalog::{
    Bsc5Entry, JUPITER_ORBIT, MARS_ORBIT, MERCURY_ORBIT, MOON_ORBIT, NEPTUNE_ORBIT, SATURN_ORBIT, URANUS_ORBIT,
    VENUS_ORBIT,
};

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

/// A catalog star.
#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub catalog_number: u32,
    /// J2000 position.
    pub catalog_position: Equatorial,
    /// Radians per year.
    pub proper_motion: Equatorial,
    pub magnitude: f32,
    pub appearance: Appearance,
    /// Whether the catalog has data for this star (a few catalog numbers are empty placeholders).
    pub has_data: bool,
    /// Apparent position, updated every frame.
    pub position: Horizontal,
}

impl Star {
    /// A star from its catalog entry and proper name.
    pub fn from_entry(entry: &Bsc5Entry, name: Option<&'static str>) -> Star {
        let glyph_index = select_star_glyph_index(entry.magnitude);
        Star {
            catalog_number: entry.catalog_number,
            catalog_position: Equatorial {
                right_ascension: entry.right_ascension,
                declination: entry.declination,
            },
            proper_motion: Equatorial {
                right_ascension: entry.ra_motion,
                declination: entry.dec_motion,
            },
            magnitude: entry.magnitude,
            appearance: Appearance {
                ascii: STAR_GLYPHS_ASCII[glyph_index],
                unicode: STAR_GLYPHS_UNICODE[glyph_index],
                label: name,
                color: select_star_color(entry.spectral_type),
            },
            has_data: entry.has_data(),
            position: Horizontal::default(),
        }
    }
}

/// The Sun or a planet. The Sun has no orbit of its own: its position is the negated position of the Earth.
#[derive(Clone, Debug, PartialEq)]
pub struct Planet {
    pub orbit: Option<&'static PlanetOrbit>,
    pub appearance: Appearance,
    pub position: Horizontal,
}

/// The Moon.
#[derive(Clone, Debug, PartialEq)]
pub struct Moon {
    pub orbit: &'static MoonOrbit,
    pub appearance: Appearance,
    pub phase: MoonPhase,
    pub position: Horizontal,
}

/// A constellation figure as segments between indices into the star table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constellation {
    pub abbreviation: &'static str,
    pub segments: Vec<[usize; 2]>,
}

/// The Sun and the planets other than the Earth, ordered from the Sun outwards.
pub fn create_planets() -> Vec<Planet> {
    let planet = |orbit, ascii, unicode, label, color| Planet {
        orbit,
        appearance: Appearance {
            ascii,
            unicode,
            label: Some(label),
            color: Some(color),
        },
        position: Horizontal::default(),
    };
    vec![
        planet(None, '@', '☉', "Sun", Color::Yellow),
        planet(Some(&MERCURY_ORBIT), '*', '☿', "Mercury", Color::White),
        planet(Some(&VENUS_ORBIT), '*', '♀', "Venus", Color::Yellow),
        planet(Some(&MARS_ORBIT), '*', '♂', "Mars", Color::Red),
        planet(Some(&JUPITER_ORBIT), '*', '♃', "Jupiter", Color::Magenta),
        planet(Some(&SATURN_ORBIT), '*', '♄', "Saturn", Color::Yellow),
        planet(Some(&URANUS_ORBIT), '*', '⛢', "Uranus", Color::Cyan),
        planet(Some(&NEPTUNE_ORBIT), '*', '♆', "Neptune", Color::Blue),
    ]
}

/// The Moon, initially new. Its Unicode glyph is chosen when it is drawn, from its phase and the direction of the Sun.
pub fn create_moon() -> Moon {
    let phase = MoonPhase::New;
    Moon {
        orbit: &MOON_ORBIT,
        appearance: Appearance {
            ascii: 'M',
            unicode: phase.glyph(true),
            label: Some("Moon"),
            color: None,
        },
        phase,
        position: Horizontal::default(),
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
    let index = crate::astro::map_float_to_int_range(
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
    fn planets_exclude_the_earth() {
        let labels: Vec<_> = create_planets()
            .iter()
            .map(|planet| planet.appearance.label.unwrap())
            .collect();
        assert_eq!(
            labels,
            [
                "Sun", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune"
            ]
        );
    }
}
