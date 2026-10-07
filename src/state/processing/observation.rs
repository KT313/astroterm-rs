//! Correction caches over prepared read-only model results. No intrinsic stellar motion is stored here.
//! Eligibility and correction indices address the borrowed working set; output stars have their own indices.
//! Separate coordinate-space snapshots prevent a corrected direction from becoming a model input.
use crate::astro::{Matrix3, Vector3, models::BodyState};
use crate::cache::{Cache, CacheConfig};
use crate::model::{
    SkyCatalog, Directions,
    CorrectionSelection,
};
use std::sync::Arc;
pub(crate) type EligibleCache = Cache<(u64, u64, f64), Vec<bool>>;
pub(crate) type RelativeCache = Cache<(u64, BodyState), (Vec<Vector3>, Vector3)>;
pub(crate) type IlluminationCache = Cache<(Vector3, Vector3), (crate::model::MoonIllumination, crate::astro::MoonPhase)>;
pub(crate) type ApparentCache = Cache<(u64, u64, u64, Vector3), Directions>;
pub(crate) type HorizontalCache = Cache<(u64, Matrix3), Directions>;

#[derive(Default)]
pub struct ObservationCache {
    pub(crate) sources: Option<(super::StageId, super::StageId, super::StageId)>,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,                         // retained identity; catalog replacement clears all dependent fields
    pub(crate) eligible: EligibleCache,                                // current drawing eligibility in working order
    pub(crate) corrections: Cache<(u64, u64), CorrectionSelection>,     // working indices retained for drawing or constellation geometry
    pub(crate) relative: RelativeCache,                                // observer-relative AU vectors in planet order, plus Moon
    pub(crate) illumination: IlluminationCache,                        // Moon lighting from relative Sun/Moon geometry
    pub(crate) apparent: ApparentCache,                                // independent aberrated directions in corrected-star/body order
    pub(crate) horizontal: HorizontalCache,                            // independent East/North/Up directions in the same order
    pub(crate) refracted: Cache<(u64, bool), Directions>,                // independent refracted horizontal directions; never fed back into motion
}

impl ObservationCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![
            self.eligible.report("Current brightness"),
            self.corrections.report("Correction selection"),
            self.relative.report("Observer subtraction"),
            self.illumination.report("Moon illumination"),
            self.apparent.report("Aberration"),
            self.horizontal.report("Horizon rotation"),
            self.refracted.report("Refraction"),
        ]
    }

    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.eligible.stats,
            self.corrections.stats,
            self.relative.stats,
            self.illumination.stats,
            self.apparent.stats,
            self.horizontal.stats,
            self.refracted.stats,
        ]
        {
            total.hits += s.hits;
            total.refreshes += s.refreshes;
            total.bypasses += s.bypasses;
        }
        total
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(ObservationCache { config, catalog, eligible, corrections, relative, illumination, apparent, horizontal, refracted });
