//! Prepare immutable star storage and resolve constellation indices once.
use crate::model::{ObservedStar, ObservedSky, SkyCatalog, PreparedCatalog, CatalogPreparation};
use super::{grid, preparation::{prepare_compact_stars, sort_stars_by_region_and_brightness, index_representative_ids, index_representative_positions, resolve_constellation_figures, collect_constellation_endpoints, collect_endpoint_ids}};
use std::collections::HashMap;
use std::sync::Arc;

use crate::catalog::{Catalog, ConstellationFigure, StarNames};


/// Prepare and sort a borrowed catalog, retaining the caller's original rows.
pub fn prepare_catalog(catalog: &Catalog) -> std::io::Result<PreparedCatalog> {
    build_catalog(
        catalog.stars.iter().cloned(),
        catalog.names.clone(),
        &catalog.hr_representatives,
        &catalog.constellations,
    )
}

/// Consume parsed catalog rows while preparing compact storage and constellation indices.
pub fn prepare_owned_catalog(catalog: Catalog) -> std::io::Result<PreparedCatalog> {
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
) -> std::io::Result<PreparedCatalog> {
    let (mut stars, mut bounds, names) = prepare_compact_stars(entries, &names)?;                 // keep valid stars in compact column storage
    let hr_by_id = index_representative_ids(representatives);       // retain the chosen star for each shared HR catalog number
    let endpoints = collect_endpoint_ids(&stars, &hr_by_id, figures);
    sort_stars_by_region_and_brightness(&mut stars, &mut bounds, &endpoints); // each star belongs to one spatial or constellation region
    drop(endpoints);

    let index_by_hr = index_representative_positions(&stars, &hr_by_id); // find those stars after sorting
    let constellations = resolve_constellation_figures(figures, &index_by_hr); // turn line endpoints into star-array indices
    let endpoint_indices = collect_constellation_endpoints(&constellations); // record the unique stars needed by the figures
    let grid = grid::build_grid(&stars, &endpoint_indices);
    let singular_count = 0; // exceptional catalogs are rejected until sparse handling is implemented
    let catalog = SkyCatalog {
        singular_count,
        stars,
        star_exceptions: Vec::new(),
        grid,
        names,
        figures: Arc::new(crate::model::ConstellationSet { figures: constellations, endpoints: endpoint_indices }),
    };
    catalog.validate_exception_support()?;
    Ok(PreparedCatalog { catalog, preparation: CatalogPreparation { motion_bounds: bounds } })
}

/// Build a zero-position sky for fixtures, including prepared catalog data.
pub fn create_sky_from_catalog(catalog: &Catalog) -> std::io::Result<ObservedSky> {
    let catalog = Arc::new(prepare_catalog(catalog)?.catalog);
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
    Ok(sky)
}


/// Validate custom definitions and derive their endpoint list once before sharing them read-only.
pub fn prepare_constellation_set(figures: Vec<crate::model::Constellation>, star_count: usize) -> std::io::Result<Arc<crate::model::ConstellationSet>> {
    if figures.iter().flat_map(|figure| figure.segments.iter().flatten()).any(|&index| index >= star_count) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "constellation index out of range"));
    }
    let endpoints = collect_constellation_endpoints(&figures);
    Ok(Arc::new(crate::model::ConstellationSet { figures, endpoints }))
}
