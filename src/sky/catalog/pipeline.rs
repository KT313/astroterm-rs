//! Prepare immutable star storage and resolve constellation indices once.
use crate::model::{ObservedStar, ObservedSky, SkyCatalog};
use super::{grid, preparation::{prepare_compact_stars, sort_stars_by_region_and_brightness, index_representative_ids, index_representative_positions, resolve_constellation_figures, collect_constellation_endpoints, collect_fast_moving_stars}};
use std::collections::HashMap;
use std::sync::Arc;

use crate::catalog::{Catalog, ConstellationFigure, StarNames};


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
    let mut stars = prepare_compact_stars(entries);                 // keep valid stars in compact column storage
    sort_stars_by_region_and_brightness(&mut stars);                // put nearby stars together, brightest first with stable ties
    let grid = grid::build_grid(&stars);

    let hr_by_id = index_representative_ids(representatives);       // retain the chosen star for each shared HR catalog number
    let index_by_hr = index_representative_positions(&stars, &hr_by_id); // find those stars after sorting
    let constellations = resolve_constellation_figures(figures, &index_by_hr); // turn line endpoints into star-array indices
    let endpoint_indices = collect_constellation_endpoints(&constellations); // record the unique stars needed by the figures
    let always_checked = collect_fast_moving_stars(&stars);        // keep stars whose motion needs individual checks
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

