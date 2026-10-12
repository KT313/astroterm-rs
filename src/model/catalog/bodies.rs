//! The solar-system tables of the pipeline rebuild (plan section 4): the Sun, the moons and the planets. Each row
//! names the body the astronomy models in `astro::models` evaluate; the frame's first step simulates every row.
//! Built once; the moon table has one row today and allows more. The planets include the Earth, whose state the
//! observer needs; the frame does not draw the observer's own planet.
use crate::astro::models::BodyId;

/// The Sun or a planet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    pub id: BodyId,
    pub name: &'static str,
}

/// A moon and the body it orbits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Satellite {
    pub id: BodyId,
    pub primary: BodyId,
    pub name: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bodies {
    pub sun: Body,
    pub moons: Vec<Satellite>,
    pub planets: Vec<Body>,                // the eight planets, ordered from the Sun outwards
}

impl Default for Bodies {
    /// The Sun, the Earth's Moon and the eight planets.
    fn default() -> Self {
        let body = |id: BodyId| Body { id, name: display_name(id) };
        Self {
            sun: body(BodyId::Sun),
            moons: vec![Satellite { id: BodyId::Moon, primary: BodyId::Earth, name: display_name(BodyId::Moon) }],
            planets: BodyId::PLANETS.into_iter().filter(|&id| id != BodyId::Sun).map(body).collect(),
        }
    }
}

/// The English name shown in labels; `BodyId::name` is the lowercase key of the accuracy tables.
fn display_name(id: BodyId) -> &'static str {
    match id {
        BodyId::Sun => "Sun", BodyId::Mercury => "Mercury", BodyId::Venus => "Venus", BodyId::Earth => "Earth", BodyId::Mars => "Mars",
        BodyId::Jupiter => "Jupiter", BodyId::Saturn => "Saturn", BodyId::Uranus => "Uranus", BodyId::Neptune => "Neptune", BodyId::Moon => "Moon",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_hold_the_sun_the_moon_and_eight_planets() {
        let bodies = Bodies::default();
        assert_eq!(bodies.sun, Body { id: BodyId::Sun, name: "Sun" });
        assert_eq!(bodies.moons, [Satellite { id: BodyId::Moon, primary: BodyId::Earth, name: "Moon" }]);
        let names: Vec<_> = bodies.planets.iter().map(|planet| planet.name).collect();
        assert_eq!(names, ["Mercury", "Venus", "Earth", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune"]);
        assert!(bodies.planets.iter().all(|planet| planet.id != BodyId::Sun && planet.id != BodyId::Moon));
    }
}
