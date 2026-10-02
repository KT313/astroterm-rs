//! The sky model: every object in the sky and its apparent position. It holds no rendering details, so any renderer
//! can draw it.
//!
//! # Stage contract (target architecture; implementation follows in phase 2)
//!
//! `simulation state -> observed sky -> camera projection -> rendering`.
//! The current [`update_sky_positions`] still combines simulation and observation. No new runtime stages or model
//! families are introduced by documenting this contract.
//!
//! - Simulation owns geometric body states in a common barycentric, equatorial J2000 frame (AU, AU/day, f64),
//!   including Sun, Earth and Moon. Until barycentric ephemerides arrive, the heliocentric origin is a documented
//!   approximation. Each evaluator has its own sample epoch, validity interval, precision/error policy and version.
//! - Observation evaluates prepared model samples at one requested frame time, composes parent-relative states at
//!   matching epochs, constructs the full observer position/velocity, and applies light-time, aberration, horizon
//!   rotation and refraction exactly once. Emission-time queries are explicit. Missing cache coverage is reported
//!   to the coordinator; it is not hidden in rendering. Fast body spin and cheap stellar motion run on demand.
//! - Camera projection maps read-only observed directions into screen coordinates and decides visibility. An optional
//!   region restricts which objects are observed, never their values. Multiple cameras can share an observed sky.
//! - Rendering chooses glyphs/pixels, colors, labels and layout from projected output. The frame loop stays visible
//!   in `main.rs`; ordinary functions and concrete state types are sufficient.
//!
//! # Model-family ownership
//!
//! Future `astro/models/{stars,planets,moons,orientation}` files/folders own their formulas, coefficients and local
//! reference tests. Stellar motion includes distance-dependent magnitude and bounds; planetary models can batch
//! bodies; lunar models declare parent dependencies; orientation owns pole/spin/shape/site geometry. Shared orbital
//! math stays in a common lower module. No model imports catalog I/O, sky orchestration, CLI or renderers.
//! Catalog parsing supplies typed inputs, stable star IDs and compact arrays. A body's stable identity and parent
//! link are independent of the theory selected to compute it. Common observation corrections have one implementation.
//!
//! A family adapter declares native origin/frame, units, time scale, precision, supported dates and errors, then
//! returns common-frame geometric states or the stellar direction/distance representation. Per-family samples may
//! have different ages, but their evaluated results must refer to the same requested epoch before composition.
//! Refreshing the Moon does not invalidate a still-valid planetary sample. A model change invalidates only its own
//! and dependent results. Tests independently cover model outputs, shared observation and camera/render integration.
//! Earth is the only supported anchor initially; other sites/bodies remain future work.

mod objects;
mod positions;

pub use objects::{Constellation, Moon, Planet, PlanetKind, Star, create_moon, create_planets};
pub use positions::{refract_sky_positions, update_sky_positions};

use std::collections::HashMap;

use crate::catalog::{Catalog, ConstellationFigure, StarNames};

/// All objects in the sky.
#[derive(Clone, Debug)]
pub struct Sky {
    /// Brightest first; equal magnitudes by descending stable ID. Placeholders are omitted.
    pub stars: Vec<Star>,
    pub names: StarNames,
    /// Prefix updated at the current epoch, also used by refraction.
    pub(crate) updated_stars: usize,
    /// The Sun and planets, ordered from the Sun outwards.
    pub planets: Vec<Planet>,
    pub moon: Moon,
    pub constellations: Vec<Constellation>,
}

impl Sky {
    /// Build the sky from the parsed catalogs. Positions are zero until updated.
    pub fn from_catalog(catalog: &Catalog) -> Sky {
        // compact and sort drawable stars, preserving the old drawing and label tie order
        let mut stars: Vec<Star> = catalog
            .stars
            .iter()
            .filter(|star| star.has_data)
            .map(Star::from_catalog_star)
            .collect();
        stars.sort_unstable_by(|a, b| a.magnitude.total_cmp(&b.magnitude).then_with(|| b.id.cmp(&a.id)));

        // resolve representatives after sorting, without changing the choice made before overrides
        let hr_by_id: HashMap<_, _> = catalog.hr_representatives.iter().map(|(&hr, &id)| (id, hr)).collect();
        let index_by_hr: HashMap<u32, usize> = stars
            .iter()
            .enumerate()
            .filter_map(|(index, star)| Some((*hr_by_id.get(&star.id)?, index)))
            .collect();
        let constellations = catalog
            .constellations
            .iter()
            .filter_map(|figure| resolve_constellation_figure(figure, &index_by_hr))
            .collect();

        Sky {
            stars,
            names: catalog.names.clone(),
            updated_stars: 0,
            planets: create_planets(),
            moon: create_moon(),
            constellations,
        }
    }

    /// Length of the drawable prefix for an inclusive magnitude threshold.
    pub fn count_bright_stars(&self, threshold: f32) -> usize {
        self.stars.partition_point(|star| star.magnitude <= threshold)
    }

    /// Resolve a star's name from the sky-owned string block.
    pub fn star_name(&self, star: &Star) -> Option<&str> {
        self.names.get(star.name)
    }

    /// The Sun, which is the first entry of `planets`.
    pub fn sun(&self) -> &Planet {
        &self.planets[0]
    }
}

/// A figure with its stars as indices into the star table. Segments with a star missing from the dataset are left
/// out, and so is a figure without any segments left.
fn resolve_constellation_figure(
    figure: &ConstellationFigure,
    index_by_hr: &HashMap<u32, usize>,
) -> Option<Constellation> {
    let segments: Vec<[usize; 2]> = figure
        .segments
        .iter()
        .filter_map(|[a, b]| Some([*index_by_hr.get(a)?, *index_by_hr.get(b)?]))
        .collect();
    (!segments.is_empty()).then_some(Constellation {
        abbreviation: figure.abbreviation,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogStar, ConstellationFigure, Designation, load_embedded_catalog};

    fn build_sky() -> Sky {
        Sky::from_catalog(&load_embedded_catalog().expect("embedded catalog loads"))
    }

    #[test]
    fn ids_survive_compaction_and_brightness_sorting() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert_eq!(
            sky.star_name(sky.stars.iter().find(|star| star.id.0 == 7001).unwrap()),
            Some("Vega")
        );
        assert!(
            sky.stars
                .iter()
                .all(|star| star.designation == Some(Designation::Hr(star.id.0 as u32)))
        );
        assert!(sky.stars.windows(2).all(|pair| pair[0].magnitude < pair[1].magnitude
            || (pair[0].magnitude == pair[1].magnitude && pair[0].id > pair[1].id)));
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let drawn: Vec<_> = sky.stars.iter().rev().map(|star| star.id.0).collect();
        assert_eq!(&drawn[..3], &[1894, 365, 3313]);
        assert_eq!(
            sky.stars[..3].iter().map(|star| star.id.0).collect::<Vec<_>>(),
            [2491, 2326, 5340]
        );
    }

    #[test]
    fn stars_keep_their_names_and_spectral_types() {
        let sky = build_sky();
        let star = |id| sky.stars.iter().find(|star| star.id.0 == id).unwrap();
        assert_eq!(
            (sky.star_name(star(2061)), &star(2061).spectral_type),
            (Some("Betelgeuse"), b"M1")
        );
        assert_eq!(
            (sky.star_name(star(5340)), &star(5340).spectral_type),
            (Some("Arcturus"), b"K1")
        );
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert!(!sky.stars.iter().any(|star| star.id.0 == 92)); // HR 92 has no data
    }

    #[test]
    fn constellations_are_matched_by_hr_number_in_any_dataset() {
        let star = |hr: Option<u32>| CatalogStar {
            id: crate::catalog::StarId(u64::from(hr.unwrap_or(100))),
            space_motion: None,
            hr,
            name: None,
            designation: None,
            right_ascension: 0.0,
            declination: 0.0,
            ra_motion: 0.0,
            dec_motion: 0.0,
            magnitude: 1.0,
            spectral_type: *b"  ",
            color_index: None,
            has_data: true,
        };
        let figure = |abbreviation, segments| ConstellationFigure { abbreviation, segments };
        let catalog = Catalog::new(
            vec![star(Some(30)), star(None), star(Some(10)), star(Some(20))],
            StarNames::default(),
            vec![
                figure("Abc", vec![[10, 20], [20, 99]]), // HR 99 isn't in the dataset
                figure("Def", vec![[98, 99]]),
            ],
        );
        let sky = Sky::from_catalog(&catalog);
        assert_eq!(sky.constellations.len(), 1);
        let [a, b] = sky.constellations[0].segments[0];
        assert_eq!((sky.stars[a].id.0, sky.stars[b].id.0), (10, 20));
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations.len(), 88);
        let [a, b] = sky.constellations[19].segments[0];
        assert_eq!((sky.stars[a].id.0, sky.stars[b].id.0), (4785, 4915));
    }
}
