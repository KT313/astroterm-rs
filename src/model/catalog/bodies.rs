//! The solar-system tables of the pipeline rebuild (plan section 4): the Sun, the moons and the planets. Each row
//! names the body the astronomy models in `astro::models` evaluate; the frame's first step simulates every row.
//! Built once; the moon table has one row today and allows more. The planets include the Earth, whose state the
//! observer needs; the frame does not draw the observer's own planet.
use crate::astro::models::BodyId;
use crate::model::{Body, Satellite, SunTable, PlanetsTable, MoonsTable};

impl Default for SunTable {
    fn default() -> Self { Self { sun: body(BodyId::Sun) } }
}

impl Default for PlanetsTable {
    fn default() -> Self {
        Self { planets: BodyId::PLANETS.into_iter().filter(|&id| id != BodyId::Sun).map(body).collect() }
    }
}

impl Default for MoonsTable {
    fn default() -> Self {
        Self { moons: vec![Satellite { id: BodyId::Moon, primary: BodyId::Earth, name: display_name(BodyId::Moon) }] }
    }
}

fn body(id: BodyId) -> Body { Body { id, name: display_name(id) } }

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
        let bodies = crate::model::Data::default();
        assert_eq!(bodies.solar_system_sun.sun, Body { id: BodyId::Sun, name: "Sun" });
        assert_eq!(bodies.solar_system_moons.moons, [Satellite { id: BodyId::Moon, primary: BodyId::Earth, name: "Moon" }]);
        let names: Vec<_> = bodies.solar_system_planets.planets.iter().map(|planet| planet.name).collect();
        assert_eq!(names, ["Mercury", "Venus", "Earth", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune"]);
        assert!(bodies.solar_system_planets.planets.iter().all(|planet| planet.id != BodyId::Sun && planet.id != BodyId::Moon));
    }
}
