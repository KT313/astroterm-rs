//! Visible cells address observed-star rows; draw order addresses visible cells. Geometry is viewport-relative.
//! Keys retain exact comparisons. Prepared figures deliberately own a catalog copy; scratch holds sort records.
use crate::cache::{Cache, CacheConfig};
use crate::model::{
    StarKey, ProjectionBodyKey as BodyKey, ConstellationKey, HorizonGeometry, Cell, DrawRecord, View,
    ProjectionViewport as Viewport, ProjectedPlanet, ProjectedMoon, ProjectedConstellation,
};
#[derive(Default)]
pub struct ProjectionCache {
    /// Reuse policy; read by processing stages and replaced only by explicit reconfiguration.
    pub(crate) config: CacheConfig,
    /// Preparation copies of source figures; used only to detect changed caller-provided figures.
    pub(crate) prepared_figures: Vec<crate::model::Constellation>,
    /// Unique sorted catalog indices for the prepared figures; rebuilt during catalog preparation.
    pub(crate) prepared_endpoints: Vec<usize>,
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
        self.bodies.invalidate();
        self.constellations.invalidate();
        self.horizon.invalidate();
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
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
crate::cache::report_fields!(ProjectionCache { config, prepared_figures, prepared_endpoints, star_candidate, order_candidate, stars, order, draw_order_scratch, bodies, constellations, horizon });

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
