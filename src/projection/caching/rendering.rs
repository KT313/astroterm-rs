//! Validate a completed regional projection before lending its data and cheap raster dependencies together.
use crate::model::{RenderProjection, ProjectionViewport, View};
use crate::state::{ProjectionCache, RegionalObservation};

/// Unlike `borrow_projected`, this handoff cannot be edited without discarding its trusted provenance.
/// Panics if projection is unfinished, invalidated, or belongs to different observation inputs.
pub fn borrow_render_projection<'a>(storage: &'a ProjectionCache, observed: RegionalObservation<'a>, view: &View, viewport: ProjectionViewport) -> RenderProjection<'a> {
    assert!(storage.regional_active, "regional projection must be completed");
    assert_eq!(storage.render_context, Some((*view, viewport)), "projection view must match completed frame");
    assert_eq!(storage.regional_owner, Some(observed.source_id()), "projection observation owner must match");
    assert!(storage.regional_catalog.as_ref().is_some_and(|catalog| std::sync::Arc::ptr_eq(catalog, observed.sky().catalog)), "projection catalog must match");
    assert_eq!(storage.regional_spans.len(), observed.regions().len(), "projection regions must match");
    for region in observed.regions() {
        let cells = &storage.regional_stars[region.region];
        let order = &storage.regional_orders[region.region];
        assert!(!cells.has_been_invalidated && !order.has_been_invalidated, "regional geometry must be valid");
        assert_eq!(order.key(), Some(&(region.selection_generation, region.motion_generation)), "drawing magnitudes must match observation");
        let key = ((region.selection_generation, region.apparent_generation, order.generation), observed.horizon_rotation(), observed.refraction_enabled(), *view, viewport);
        assert_eq!(cells.key(), Some(&key), "projected directions must match observation and draw order");
    }
    for span in &storage.regional_spans {
        let region = &observed.regions()[span.slot];
        assert_eq!((span.region, span.generation), (region.region, storage.regional_stars[region.region].generation), "spans must match current regions");
    }
    RenderProjection { sky: super::borrow_projected(storage, observed.sky(), view, viewport),
        source: (storage.render_source_id(), storage.source_revision), regions: observed.regions,
        spans: &storage.regional_spans,
        geometry: [storage.bodies.generation, storage.constellations.generation, storage.horizon.generation] }
}
