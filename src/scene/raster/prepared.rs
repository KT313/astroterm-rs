//! Catalog-bound display constants prepared once before the frame loop. A strong owner prevents address reuse;
//! callers rendering another catalog use the original formulas instead of reading stale constants.
use crate::canvas::Color;
use crate::model::{SkyCatalog, ObservedStarView};
use crate::timing::StepTimes;
use std::sync::Arc;
use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};


use crate::model::{PreparedScene, StarDisplay};
/// Prepare catalog-bound star colors and name eligibility without changing dynamic brightness.
pub(crate) fn prepare_scene(catalog: Arc<SkyCatalog>, times: &mut StepTimes) -> PreparedScene {
    let stars: Vec<_> = times.measure("Star display constants", || {
        (0..catalog.stars.len())
            .map(|i| {
                let spectral = catalog.stars.spectral_type(i);
                let color_index = catalog.stars.color_index(i);
                StarDisplay {
                    rgb: super::pixels::compute_star_rgb(spectral, color_index),
                    color: super::appearance::select_star_color(spectral, color_index),
                    named: catalog.stars.name(i).is_some(),
                }
            })
            .collect()
    });
    {
        times.record_memory(times.last_memory_step(), || MemoryEvent::borrow(BufferId::CatalogStars, Access::ReadOnly, BufferShape::unknown(IndexDomain::Catalog)));
        times.record_shape(BufferId::PreparedDisplay, Operation::Build, None, || BufferShape::vector(&stars, IndexDomain::Catalog));
    }
    times.describe("Star display constants", || format!("stars={}; named candidates={}; prepared RGB/character color/name-eligibility bytes={}; brightness-dependent rendering remains dynamic", stars.len(), stars.iter().filter(|s| s.named).count(), stars.len()*std::mem::size_of::<StarDisplay>()));
    PreparedScene { catalog, stars }
}

fn find_display<'a>(prepared: &'a PreparedScene, star: &ObservedStarView<'_>) -> Option<&'a StarDisplay> {
    std::ptr::eq(star.catalog, &prepared.catalog.stars).then(|| &prepared.stars[star.source_index])
}

pub(crate) fn resolve_prepared_rgb(prepared: &PreparedScene, star: &ObservedStarView<'_>) -> [u8; 3] {
    find_display(prepared, star).map_or_else(|| super::pixels::star_rgb(star), |s| s.rgb)
}

pub(crate) fn resolve_prepared_color(prepared: &PreparedScene, star: &ObservedStarView<'_>) -> Option<Color> {
    find_display(prepared, star).map_or_else(
        || super::appearance::select_star_color(star.spectral_type(), star.color_index()),
        |s| s.color,
    )
}

pub(crate) fn is_prepared_star_named(prepared: &PreparedScene, star: &ObservedStarView<'_>) -> bool {
    find_display(prepared, star).map_or_else(|| star.name().is_some(), |s| s.named)
}

pub(crate) fn resolve_star_rgb(star: &ObservedStarView<'_>, prepared: Option<&PreparedScene>) -> [u8; 3] {
    prepared.map_or_else(|| super::pixels::star_rgb(star), |p| resolve_prepared_rgb(p, star))
}
