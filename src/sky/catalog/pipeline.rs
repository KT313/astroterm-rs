//! Prepare immutable star storage and resolve constellation indices once.
use crate::model::{SkyCatalog, PreparedCatalog, CatalogPreparation};
use super::{grid, preparation::{prepare_compact_stars, sort_stars_by_region_and_brightness, create_sort_pool, index_representative_ids, index_representative_positions, resolve_constellation_figures, collect_constellation_endpoints, collect_endpoint_ids}};
use std::collections::HashMap;
use std::sync::Arc;
use crate::timing::StepTimes;
use crate::progress::{DatasetProgress, Report, Stage, Unit, Outcome};

use crate::catalog::{Catalog, ConstellationFigure, StarNames};


/// Prepare and sort a borrowed catalog, retaining the caller's original rows.
pub fn prepare_catalog(catalog: &Catalog, report: &mut Report<'_>) -> std::io::Result<PreparedCatalog> {
    let (progress, times) = report.parts();
    build_catalog(catalog.stars.iter().cloned(), catalog.names.clone(), &catalog.hr_representatives, &catalog.constellations, progress, times)
}

/// Consume parsed catalog rows while preparing compact storage and constellation indices.
pub fn prepare_owned_catalog(catalog: Catalog, report: &mut Report<'_>) -> std::io::Result<PreparedCatalog> {
    let (progress, times) = report.parts();
    build_catalog(
        catalog.stars.into_iter(),
        catalog.names,
        &catalog.hr_representatives,
        &catalog.constellations,
        progress,
        times,
    )
}

fn build_catalog(entries: impl Iterator<Item = crate::catalog::CatalogStar>, names: StarNames,
    representatives: &HashMap<u32, crate::catalog::StarId>, figures: &[ConstellationFigure], progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> std::io::Result<PreparedCatalog> {
    let (stars, bounds, names) = times.measure("Compact star preparation", || prepare_compact_stars(entries, &names, progress))?; // convert this parser's rows into compact storage
    finish_catalog(stars, bounds, names, representatives, figures, progress, times)
}

pub(super) fn finish_catalog(stars: crate::model::StarStorage, bounds: Vec<f32>, names: StarNames,
    representatives: &HashMap<u32, crate::catalog::StarId>, figures: &[ConstellationFigure], progress: &mut dyn DatasetProgress, times: &mut StepTimes) -> std::io::Result<PreparedCatalog> {
    let pool = times.measure("Startup sort workers", || create_sort_pool(stars.len()))?;
    times.describe("Startup sort workers", || format!("worker threads={}; parallel sort threshold={} rows", pool.as_ref().map_or(0, rayon::ThreadPool::current_num_threads), crate::constants::STARTUP_PARALLEL_SORT_ROWS));
    finish_catalog_with_pool(stars, bounds, names, representatives, figures, progress, times, pool.as_ref())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_catalog_with_pool(mut stars: crate::model::StarStorage, mut bounds: Vec<f32>, names: StarNames,
    representatives: &HashMap<u32, crate::catalog::StarId>, figures: &[ConstellationFigure], progress: &mut dyn DatasetProgress,
    times: &mut StepTimes, pool: Option<&rayon::ThreadPool>) -> std::io::Result<PreparedCatalog> {
    progress.begin_stage(Stage::AssignRegions, "", Unit::Work, None);
    progress.set_detail("Resolving constellation membership");
    let hr_by_id = index_representative_ids(representatives);       // retain the chosen star for each shared HR catalog number
    let endpoints = times.measure("Constellation membership", || collect_endpoint_ids(&stars, &hr_by_id, figures));
    let counts = sort_stars_by_region_and_brightness(&mut stars, &mut bounds, &endpoints, progress, times, pool)?;
    drop(endpoints);

    progress.begin_stage(Stage::Index, "Reusing region counts", Unit::Work, None);
    let (constellations, endpoint_indices) = times.measure("Constellation index resolution", || {
        let index_by_hr = index_representative_positions(&stars, &hr_by_id);
        let constellations = resolve_constellation_figures(figures, &index_by_hr);
        let endpoints = collect_constellation_endpoints(&constellations);
        (constellations, endpoints)
    });
    let grid = times.measure("Sky index construction", || grid::build_grid_from_counts(stars.len(), counts, &endpoint_indices, progress))?;
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
    progress.finish_stage(Outcome::Complete);
    Ok(PreparedCatalog { catalog, preparation: CatalogPreparation { motion_bounds: bounds } })
}

/// Validate custom definitions and derive their endpoint list once before sharing them read-only.
pub fn prepare_constellation_set(figures: Vec<crate::model::Constellation>, star_count: usize) -> std::io::Result<Arc<crate::model::ConstellationSet>> {
    if figures.iter().flat_map(|figure| figure.segments.iter().flatten()).any(|&index| index >= star_count) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "constellation index out of range"));
    }
    let endpoints = collect_constellation_endpoints(&figures);
    Ok(Arc::new(crate::model::ConstellationSet { figures, endpoints }))
}
