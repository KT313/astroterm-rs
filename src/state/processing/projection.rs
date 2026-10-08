//! Regional geometry retains catalog indices; frame cells resolve them to observed rows before rendering.
//! Exact whole-sky caches remain available for caller-editable headless input; all geometry is viewport-relative.
use crate::cache::{Cache, CacheConfig};
use crate::model::{
    RegionalProjectionKey, RegionalOrderKey, RegionalDrawRecord, StarKey, ProjectionBodyKey as BodyKey, ConstellationKey, HorizonGeometry, Cell, DrawRecord, View,
    ProjectionViewport as Viewport, ProjectedPlanet, ProjectedMoon, ProjectedConstellation,
};
#[derive(Default)]
pub struct ProjectionCache {
    /// Cells retain catalog indices; order offsets are local to a region and guarded by its membership version.
    pub(crate) regional_stars: Vec<Cache<RegionalProjectionKey, Vec<(usize, Cell)>>>,
    pub(crate) regional_orders: Vec<Cache<RegionalOrderKey, Vec<RegionalDrawRecord>>>,
    pub(crate) regional_catalog: Option<std::sync::Arc<crate::model::SkyCatalog>>,
    pub(crate) regional_owner: Option<u64>,
    pub(crate) regional_active: bool,
    pub(crate) regional_stats: crate::cache::CacheStats,
    pub(crate) regional_cells: Vec<(usize, Cell)>,
    pub(crate) regional_ranges: Vec<(usize, usize)>, // start/end of each region in directly drawable cells
    pub(crate) region_cell_scratch: Vec<Option<Cell>>, // temporary cells indexed within one observed region
    pub(crate) assembled_for: Vec<(usize, usize, usize, u64, u64)>, // region, current observed row range, cell version, order version
    pub(crate) assembly_valid: bool,
    /// Reuse policy; read by processing stages and replaced only by explicit reconfiguration.
    pub(crate) config: CacheConfig,
    /// Exact candidate key in observed order; cleared on a hit, transferred on successful refresh.
    pub(crate) star_candidate: StarKey,
    /// Observed index, current magnitude and ID for each visible cell; same candidate lifecycle.
    pub(crate) order_candidate: Vec<(usize, f64, crate::catalog::StarId)>,
    /// Visible observed indices and signed row/column cells, in observed order. Refreshed on geometry changes.
    pub(crate) stars: Cache<StarKey, Vec<(usize, Cell)>>,
    /// Dimmest-first indices into visible cells, ties by ascending stable ID. No catalog-index substitution.
    pub(crate) order: Cache<Vec<(usize, f64, crate::catalog::StarId)>, Vec<usize>>,
    /// Temporary comparison records indexed by visible position; retained capacity across sorts.
    pub(crate) draw_order_scratch: Vec<DrawRecord>,
    /// Projected Sun/planet cells and lunar display geometry; includes hidden records.
    pub(crate) bodies: Cache<BodyKey, (Vec<ProjectedPlanet>, ProjectedMoon)>,
    /// Clipped constellation arcs in viewport cells/pixels; independent of draw toggle.
    pub(crate) constellations: Cache<ConstellationKey, Vec<ProjectedConstellation>>,
    /// Projected horizon segments and orientation label origins; dependent only on view and viewport.
    pub(crate) horizon: Cache<(View, Viewport), HorizonGeometry>,
}
impl ProjectionCache {
    /// Borrow only the candidate and committed visible-cell cache for star projection.
    pub(crate) fn star_buffers(&mut self) -> StarProjectionBuffers<'_> {
        StarProjectionBuffers { candidate: &mut self.star_candidate, cells: &mut self.stars }
    }

    /// Read projected membership while changing only draw-order storage.
    pub(crate) fn order_buffers(&mut self) -> DrawOrderBuffers<'_> {
        DrawOrderBuffers { cells: self.stars.value(), candidate: &mut self.order_candidate, order: &mut self.order, scratch: &mut self.draw_order_scratch }
    }

    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn invalidate_view(&mut self) {
        self.stars.invalidate();
        for cache in &mut self.regional_stars { cache.invalidate(); }
        self.bodies.invalidate();
        self.constellations.invalidate();
        self.horizon.invalidate();
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.regional_stats,
            self.stars.stats,
            self.order.stats,
            self.bodies.stats,
            self.constellations.stats,
            self.horizon.stats,
        ] {
            total.hits += s.hits;
            total.refreshes += s.refreshes;
            total.bypasses += s.bypasses;
        }
        total
    }
}
#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for ProjectionCache {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        use super::support::regions::{report_region_storage, cached_vector_bytes};
        report_region_storage(sink, "regional_stars", &self.regional_stars, cached_vector_bytes);
        report_region_storage(sink, "regional_orders", &self.regional_orders, cached_vector_bytes);
        crate::cache::report_field(sink, "regional_catalog", &self.regional_catalog);
        crate::cache::report_field(sink, "regional_owner", &self.regional_owner);
        crate::cache::report_field(sink, "regional_active", &self.regional_active);
        crate::cache::report_field(sink, "regional_cells", &self.regional_cells);
        crate::cache::report_field(sink, "regional_ranges", &self.regional_ranges);
        crate::cache::report_field(sink, "region_cell_scratch", &self.region_cell_scratch);
        crate::cache::report_field(sink, "assembled_for", &self.assembled_for);
        crate::cache::report_field(sink, "assembly_valid", &self.assembly_valid);
        crate::cache::report_field(sink, "config", &self.config);
        crate::cache::report_field(sink, "star_candidate", &self.star_candidate);
        crate::cache::report_field(sink, "order_candidate", &self.order_candidate);
        crate::cache::report_field(sink, "stars", &self.stars);
        crate::cache::report_field(sink, "order", &self.order);
        crate::cache::report_field(sink, "draw_order_scratch", &self.draw_order_scratch);
        crate::cache::report_field(sink, "bodies", &self.bodies);
        crate::cache::report_field(sink, "constellations", &self.constellations);
        crate::cache::report_field(sink, "horizon", &self.horizon);
    }
}

/// Projection may append cells and update its cache, but cannot access motion or rendering state.
pub(crate) struct StarProjectionBuffers<'a> {
    pub candidate: &'a mut StarKey,
    pub cells: &'a mut Cache<StarKey, Vec<(usize, Cell)>>,
}

/// Each index in `order` addresses `cells`; the cell's index addresses the observed-star array.
pub(crate) struct DrawOrderBuffers<'a> {
    pub cells: &'a [(usize, Cell)],
    pub candidate: &'a mut Vec<(usize, f64, crate::catalog::StarId)>,
    pub order: &'a mut Cache<Vec<(usize, f64, crate::catalog::StarId)>, Vec<usize>>,
    pub scratch: &'a mut Vec<DrawRecord>,
}
