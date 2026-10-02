//! Celestial objects: what they are and where they are. How they look is up to the renderer.

use crate::astro::{Equatorial, Horizontal, MoonOrbit, MoonPhase, PlanetOrbit};
use crate::catalog::{
    Bsc5Entry, JUPITER_ORBIT, MARS_ORBIT, MERCURY_ORBIT, MOON_ORBIT, NEPTUNE_ORBIT, SATURN_ORBIT, URANUS_ORBIT,
    VENUS_ORBIT,
};

/// A catalog star.
#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub catalog_number: u32,
    /// Proper name, for the brighter stars that have one.
    pub name: Option<&'static str>,
    /// J2000 position.
    pub catalog_position: Equatorial,
    /// Radians per year.
    pub proper_motion: Equatorial,
    pub magnitude: f32,
    /// Morgan-Keenan spectral class and subclass as in the catalog, e.g. `*b"K1"`.
    pub spectral_type: [u8; 2],
    /// Whether the catalog has data for this star (a few catalog numbers are empty placeholders).
    pub has_data: bool,
    /// Apparent position, updated every frame.
    pub position: Horizontal,
}

impl Star {
    /// A star from its catalog entry and proper name.
    pub fn from_entry(entry: &Bsc5Entry, name: Option<&'static str>) -> Star {
        Star {
            catalog_number: entry.catalog_number,
            name,
            catalog_position: Equatorial {
                right_ascension: entry.right_ascension,
                declination: entry.declination,
            },
            proper_motion: Equatorial {
                right_ascension: entry.ra_motion,
                declination: entry.dec_motion,
            },
            magnitude: entry.magnitude,
            spectral_type: entry.spectral_type,
            has_data: entry.has_data(),
            position: Horizontal::default(),
        }
    }
}

/// Which of the Sun and the planets an entry of [`Sky::planets`](super::Sky::planets) is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetKind {
    Sun,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl PlanetKind {
    /// The Sun and the planets other than the Earth, ordered from the Sun outwards.
    pub const ALL: [PlanetKind; 8] = [
        PlanetKind::Sun,
        PlanetKind::Mercury,
        PlanetKind::Venus,
        PlanetKind::Mars,
        PlanetKind::Jupiter,
        PlanetKind::Saturn,
        PlanetKind::Uranus,
        PlanetKind::Neptune,
    ];

    /// English name, e.g. "Jupiter".
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 8] = [
            "Sun", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune",
        ];
        NAMES[self as usize]
    }

    /// Heliocentric orbit. The Sun has none: its position is the negated position of the Earth.
    pub fn orbit(self) -> Option<&'static PlanetOrbit> {
        match self {
            PlanetKind::Sun => None,
            PlanetKind::Mercury => Some(&MERCURY_ORBIT),
            PlanetKind::Venus => Some(&VENUS_ORBIT),
            PlanetKind::Mars => Some(&MARS_ORBIT),
            PlanetKind::Jupiter => Some(&JUPITER_ORBIT),
            PlanetKind::Saturn => Some(&SATURN_ORBIT),
            PlanetKind::Uranus => Some(&URANUS_ORBIT),
            PlanetKind::Neptune => Some(&NEPTUNE_ORBIT),
        }
    }
}

/// The Sun or a planet.
#[derive(Clone, Debug, PartialEq)]
pub struct Planet {
    pub kind: PlanetKind,
    pub position: Horizontal,
}

/// The Moon.
#[derive(Clone, Debug, PartialEq)]
pub struct Moon {
    pub orbit: &'static MoonOrbit,
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
    PlanetKind::ALL
        .iter()
        .map(|&kind| Planet {
            kind,
            position: Horizontal::default(),
        })
        .collect()
}

/// The Moon, initially new.
pub fn create_moon() -> Moon {
    Moon {
        orbit: &MOON_ORBIT,
        phase: MoonPhase::New,
        position: Horizontal::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planets_exclude_the_earth() {
        let names: Vec<_> = create_planets().iter().map(|planet| planet.kind.name()).collect();
        assert_eq!(
            names,
            [
                "Sun", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune"
            ]
        );
    }

    #[test]
    fn only_the_sun_has_no_orbit() {
        let without_orbit: Vec<_> = PlanetKind::ALL
            .into_iter()
            .filter(|kind| kind.orbit().is_none())
            .collect();
        assert_eq!(without_orbit, [PlanetKind::Sun]);
    }
}
