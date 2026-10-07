//! Celestial objects: what they are and where they are. How they look is up to the renderer.
use crate::rows::row_columns;
use crate::astro::models::stars::StellarMotion;
use crate::astro::{Horizontal, MoonPhase, Vector3};
use crate::catalog::{NameId, StarId};

/// A catalog star.
#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub id: StarId,
    /// Display label: proper name when available, otherwise a catalog identifier.
    pub name: Option<NameId>,
    pub motion: StellarMotion,
    /// Catalog magnitude at J2000; evaluated magnitude belongs to ObservedStar.
    pub magnitude: f64,
    pub brightness_key: f64,
    pub display_color: crate::model::StarColor,
    /// Whether the catalog has data for this star (a few catalog numbers are empty placeholders).
    pub has_data: bool,
}

/// Per-frame output for drawable candidates and required constellation endpoints, in catalog-index order.
/// `drawable` uses the current magnitude; immutable model inputs stay in SkyCatalog.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedStar {
    pub source_index: usize,
    pub drawable: bool,
    pub magnitude: f64,
    /// Unit horizontal direction: East, North, Up; observer corrections have already been applied.
    pub position: Vector3,
}
row_columns!(ObservedStar { source_index, drawable, magnitude, position });
impl ObservedStar {
    pub fn horizontal_position(&self) -> Horizontal {
        Horizontal::from_vector(self.position)
    }

    /// Create calculated state for `star` at its index in the associated prepared catalog.
    /// Metadata is resolved from that catalog, so callers must keep the index/catalog association intact.
    pub fn from_star(star: &Star, source_index: usize, position: Vector3) -> Self {
        Self {
            source_index,
            drawable: true,
            magnitude: star.magnitude,
            position,
        }
    }
}

/// Borrowed read-only metadata with a separate calculated state. No catalog fields are expanded or copied
/// when this view is created. The owning sky keeps the immutable catalog alive.
#[derive(Clone, Copy)]
pub struct ObservedStarView<'a> {
    pub state: &'a ObservedStar,
    pub catalog: &'a crate::model::StarStorage,
}
impl std::fmt::Debug for ObservedStarView<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservedStarView")
            .field("state", self.state)
            .field("id", &self.id())
            .field("name", &self.name())
            .field("display_color", &self.display_color())
            .finish()
    }
}
impl std::ops::Deref for ObservedStarView<'_> {
    type Target = ObservedStar;
    fn deref(&self) -> &ObservedStar {
        self.state
    }
}
impl PartialEq for ObservedStarView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.id() == other.id()
            && self.name() == other.name()
            && self.display_color() == other.display_color()
    }
}
impl ObservedStarView<'_> {
    pub fn id(&self) -> StarId {
        self.catalog.id(self.source_index)
    }
    pub fn name(&self) -> Option<NameId> {
        self.catalog.name(self.source_index)
    }
    pub fn display_color(&self) -> crate::model::StarColor {
        self.catalog.display_color(self.source_index)
    }
    pub fn has_data(&self) -> bool {
        true // prepared catalogs contain no placeholders
    }
}

/// Which of the Sun and the planets an entry of [`Sky::planets`](crate::model::Sky::planets) is.
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
}

/// The Sun or a planet.
#[derive(Clone, Debug, PartialEq)]
pub struct Planet {
    pub kind: PlanetKind,
    /// Unit horizontal direction: East, North, Up; observer corrections have already been applied.
    pub position: Vector3,
}
row_columns!(Planet { kind, position });

/// The Moon.
#[derive(Clone, Debug, PartialEq)]
pub struct Moon {
    pub phase: MoonPhase,
    pub illumination: crate::model::MoonIllumination,
    /// Unit horizontal direction: East, North, Up; observer corrections have already been applied.
    pub position: Vector3,
}
row_columns!(Moon { phase, illumination, position });

impl Planet {
    pub fn horizontal_position(&self) -> Horizontal {
        Horizontal::from_vector(self.position)
    }
}
impl Moon {
    pub fn horizontal_position(&self) -> Horizontal {
        Horizontal::from_vector(self.position)
    }
}

/// A constellation figure as segments between indices into the star table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constellation {
    pub abbreviation: &'static str,
    pub segments: Vec<[usize; 2]>,
}
row_columns!(Constellation { abbreviation, segments });

/// The Sun and the planets other than the Earth, ordered from the Sun outwards.
pub fn create_planets() -> Vec<Planet> {
    PlanetKind::ALL
        .iter()
        .map(|&kind| Planet {
            kind,
            position: Horizontal::default().to_unit_vector(),
        })
        .collect()
}

/// The Moon, initially new.
pub fn create_moon() -> Moon {
    Moon {
        phase: MoonPhase::New,
        illumination: crate::model::MoonIllumination::default(),
        position: Horizontal::default().to_unit_vector(),
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
}
