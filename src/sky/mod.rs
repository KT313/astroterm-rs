//! Four-stage pipeline: immutable catalog and independently cached geometric simulation -> observer-relative sky
//! -> camera projection -> rendering. Astronomy families live in astro::models; no camera enters observation.
//! Common states use f64 J2000 equatorial AU/AU-day, with a barycentric origin. Earth is the only real
//! anchor, with a WGS84 sea-level site. Observation applies light-time, exact parallax and aberration before
//! horizon rotation and optional refraction; camera projection never changes these values.

pub mod cache;
pub mod grid;
mod illumination;
mod objects;
pub use objects::prepare_star;
pub use illumination::{compute_moon_illumination, name_moon_phase};
use crate::model::storage::StarStorage;
mod observation;
mod positions;
pub mod simulation;
pub use observation::cached::{prepare_observation_catalog, prepare_cached_observer, prepare_cached_observer_with_times, prepare_cached_light_time, observe_cached_sky};
pub use observation::{compose_observer_state, observe_sky, observe_sky_candidates, prepare_light_time_samples, prepare_observation, prepare_observer};
pub use simulation::update_simulation;

use crate::model::objects::{Constellation, ObservedStar};
pub use positions::{refract_sky_positions, update_sky_positions};

use std::collections::HashMap;
use std::sync::Arc;

use crate::catalog::{Catalog, ConstellationFigure, StarNames};

use crate::model::{ObservedSky, SkyCatalog};

/// Prepare and sort a borrowed catalog, retaining the caller's original rows.
pub fn prepare_catalog(catalog: &Catalog) -> SkyCatalog {
    build_catalog(
        catalog.stars.iter().cloned(),
        catalog.names.clone(),
        &catalog.hr_representatives,
        &catalog.constellations,
    )
}

/// Consume parsed catalog rows while preparing compact storage and constellation indices.
pub fn prepare_owned_catalog(catalog: Catalog) -> SkyCatalog {
    build_catalog(
        catalog.stars.into_iter(),
        catalog.names,
        &catalog.hr_representatives,
        &catalog.constellations,
    )
}

fn build_catalog(
    entries: impl Iterator<Item = crate::catalog::CatalogStar>,
    names: StarNames,
    representatives: &HashMap<u32, crate::catalog::StarId>,
    figures: &[ConstellationFigure],
) -> SkyCatalog {
    // prepare directly into compact arrays, then permute in place by cell, key and stable ID
    let mut stars = StarStorage::default();
    stars.reserve(entries.size_hint().1.unwrap_or(0));
    for entry in entries.filter(|s| s.has_data) {
        stars.push(prepare_star(&entry));
    }
    stars.shrink_to_fit();
    let cells: Vec<_> = (0..stars.len()).map(|i| grid::stored_cell(&stars, i)).collect();
    let mut order: Vec<_> = (0..stars.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        cells[a]
            .cmp(&cells[b])
            .then_with(|| stars.brightness_key(a).total_cmp(&stars.brightness_key(b)))
            .then_with(|| stars.id(b).cmp(&stars.id(a)))
    });
    stars.reorder(&order);
    drop(order);
    drop(cells);
    let grid = grid::build_grid(&stars);

    // resolve representatives after sorting, without changing the choice made before overrides
    let hr_by_id: HashMap<_, _> = representatives.iter().map(|(&hr, &id)| (id, hr)).collect();
    let index_by_hr: HashMap<u32, usize> = stars
        .iter()
        .enumerate()
        .filter_map(|(index, star)| Some((*hr_by_id.get(&star.id)?, index)))
        .collect();
    let constellations: Vec<Constellation> = figures
        .iter()
        .filter_map(|figure| resolve_constellation_figure(figure, &index_by_hr))
        .collect();

    let mut endpoint_indices: Vec<_> = constellations
        .iter()
        .flat_map(|figure| figure.segments.iter().flatten())
        .copied()
        .collect();
    endpoint_indices.sort_unstable();
    endpoint_indices.dedup();
    let always_checked: Vec<_> = stars
        .iter()
        .enumerate()
        .filter_map(|(i, star)| {
            (star.motion_bound > crate::astro::models::stars::ALWAYS_CHECKED_ANGLE).then_some(i)
        })
        .collect();
    let singular_count = stars.iter().filter(|star| star.singular_fallback).count();
    SkyCatalog {
        endpoint_indices: endpoint_indices.into(),
        always_checked: always_checked.into(),
        singular_count,
        stars,
        grid,
        names,
        constellations,
    }
}

/// Build a zero-position sky for fixtures, including prepared catalog data.
pub fn create_sky_from_catalog(catalog: &Catalog) -> ObservedSky {
    let catalog = Arc::new(prepare_catalog(catalog));
    let mut sky = ObservedSky::new(catalog);
    sky.stars = sky
        .catalog
        .stars
        .iter()
        .enumerate()
        .map(|(index, star)| {
            ObservedStar::from_star(&star, index, crate::astro::Horizontal::default().to_unit_vector())
        })
        .collect();
    sky.corrections.evaluated = sky.stars.len();
    sky
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
    use crate::model::Sky;
    use super::*;
    use crate::catalog::{CatalogStar, ConstellationFigure, Designation, load_embedded_catalog};

    fn build_sky() -> Sky {
        crate::sky::create_sky_from_catalog(&load_embedded_catalog().expect("embedded catalog loads"))
    }

    #[test]
    fn ids_survive_compaction_and_brightness_sorting() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert_eq!(
            sky.star_name(&sky.star_views().find(|star| star.id().0 == 7001).unwrap()),
            Some("Vega")
        );
        assert!(
            sky.star_views()
                .all(|star| star.designation().resolve() == Some(Designation::Hr(star.id().0 as u32)))
        );
        for range in sky.catalog.grid.offsets.windows(2) {
            let stars = &sky.catalog.stars;
            for i in range[0] + 1..range[1] {
                assert!(
                    stars.brightness_key(i - 1) < stars.brightness_key(i)
                        || (stars.brightness_key(i - 1) == stars.brightness_key(i) && stars.id(i - 1) > stars.id(i))
                );
            }
        }
    }

    #[test]
    fn stars_are_drawn_dimmest_first() {
        let sky = build_sky();
        let projected_data = crate::projection::project_sky(
            &sky,
            &crate::model::projection::View::default(),
            crate::model::projection::ProjectionViewport { height: 41, width: 81 },
        );
        let projected = projected_data.view(&sky);
        let drawn: Vec<_> = projected.stars.iter().map(|s| s.star.id().0).collect();
        assert_eq!(&drawn[..3], &[1894, 365, 3313]);
        assert_eq!(
            drawn.iter().rev().take(3).copied().collect::<Vec<_>>(),
            [2491, 2326, 5340]
        );
    }

    #[test]
    fn stars_keep_their_names_and_spectral_types() {
        let sky = build_sky();
        let star = |id| sky.star_views().find(|star| star.id().0 == id).unwrap();
        assert_eq!(
            (sky.star_name(&star(2061)), &star(2061).spectral_type()),
            (Some("Betelgeuse"), b"M1")
        );
        assert_eq!(
            (sky.star_name(&star(5340)), &star(5340).spectral_type()),
            (Some("Arcturus"), b"K1")
        );
    }

    #[test]
    fn placeholder_stars_are_never_drawn() {
        let sky = build_sky();
        assert_eq!(sky.stars.len(), 9110 - 14);
        assert!(!sky.star_views().any(|star| star.id().0 == 92)); // HR 92 has no data
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
            ra_motion_cos_dec: 0.0,
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
        let sky = crate::sky::create_sky_from_catalog(&catalog);
        assert_eq!(sky.constellations.len(), 1);
        let [a, b] = sky.constellations[0].segments[0];
        assert_eq!((sky.star_view(a).id().0, sky.star_view(b).id().0), (10, 20));
    }

    #[test]
    fn constellations_reference_star_indices() {
        let sky = build_sky();
        assert_eq!(sky.constellations.len(), 88);
        let [a, b] = sky.constellations[19].segments[0];
        assert_eq!((sky.star_view(a).id().0, sky.star_view(b).id().0), (4785, 4915));
    }
}
