//! Correction caches over prepared read-only model results. No intrinsic stellar motion is stored here.
//! Regional slots retain stable catalog indices. Combined flags/indices keep the current working order for existing consumers.
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
/// Each retained result addresses stable catalog rows within one region.
#[derive(Default)]
pub(crate) struct ObservationRegion {
    pub eligible: EligibleCache,
    pub corrections: Cache<(u64, u64), (Vec<crate::model::SelectedStar>, crate::model::CorrectionStats)>,
    pub apparent: Cache<(u64, u64, Vector3), Vec<Vector3>>,
}
pub(crate) type BodyApparentCache = Cache<(u64, Vector3), (Vec<Vector3>, Vector3)>;
pub(crate) type HorizontalCache = Cache<(u64, Matrix3), Directions>;

#[derive(Default)]
pub struct ObservationCache {
    pub(crate) identity: super::StageId,
    pub(crate) regions: Vec<ObservationRegion>,
    pub(crate) region_stats: [crate::cache::CacheStats; 3],
    pub(crate) body_apparent: BodyApparentCache,
    pub(crate) regional_output: Vec<crate::model::ObservedRegion>, // frame-local ranges; never persisted inside regional results
    pub(crate) sources: Option<(super::StageId, super::StageId, super::StageId)>,
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,                         // retained identity; catalog replacement clears all dependent fields
    pub(crate) eligible: EligibleCache,                                // current drawing eligibility in working order
    pub(crate) corrections: Cache<(u64, u64), CorrectionSelection>,     // working indices retained for drawing or constellation geometry
    pub(crate) relative: RelativeCache,                                // observer-relative AU vectors in planet order, plus Moon
    pub(crate) illumination: IlluminationCache,                        // Moon lighting from relative Sun/Moon geometry
    pub(crate) apparent: ApparentCache,                                // assembled directions required by the whole-sky horizon/refraction caches
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

    /// Inspect a regional result without making it valid for the current request.
    pub fn region_reports(&self, region: usize) -> Option<[crate::cache::CacheReport; 3]> {
        let region = self.regions.get(region)?;
        Some([region.eligible.report("Current brightness"), region.corrections.report("Correction selection"), region.apparent.report("Aberration")])
    }

    pub fn invalidate_region(&mut self, region: usize) {
        let region = self.regions.get_mut(region).expect("known observation region");
        region.eligible.invalidate();
        region.corrections.invalidate();
        region.apparent.invalidate();
        self.eligible.invalidate();
        self.corrections.invalidate();
        self.apparent.invalidate();
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
        for stats in self.region_stats {
            total.hits += stats.hits;
            total.refreshes += stats.refreshes;
            total.bypasses += stats.bypasses;
        }
        total.hits += self.body_apparent.stats.hits;
        total.refreshes += self.body_apparent.stats.refreshes;
        total.bypasses += self.body_apparent.stats.bypasses;
        total
    }
}
#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for ObservationCache {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        super::support::regions::report_region_storage(sink, "regions", &self.regions, |region| {
            let sizes = crate::state::observation_region_bytes(region);
            Some((sizes.used?, sizes.reserved?))
        });
        crate::cache::report_field(sink, "body_apparent", &self.body_apparent);
        crate::cache::report_field(sink, "regional_output", &self.regional_output);
        crate::cache::report_field(sink, "config", &self.config);
        crate::cache::report_field(sink, "catalog", &self.catalog);
        crate::cache::report_field(sink, "eligible", &self.eligible);
        crate::cache::report_field(sink, "corrections", &self.corrections);
        crate::cache::report_field(sink, "relative", &self.relative);
        crate::cache::report_field(sink, "illumination", &self.illumination);
        crate::cache::report_field(sink, "apparent", &self.apparent);
        crate::cache::report_field(sink, "horizontal", &self.horizontal);
        crate::cache::report_field(sink, "refracted", &self.refracted);
    }
}

/// Read-only handoff constructed by observation. Holding it prevents mutation of its sky and region versions.
#[derive(Clone, Copy)]
pub struct RegionalObservation<'a> {
    pub(crate) sky: &'a crate::model::ObservedSky,
    pub(crate) regions: &'a [crate::model::ObservedRegion],
    pub(crate) owner: super::StageId,
    pub(crate) horizon: Matrix3,
    pub(crate) refraction: bool,
}
impl RegionalObservation<'_> {
    pub fn sky(&self) -> &crate::model::ObservedSky { self.sky }
    pub fn regions(&self) -> &[crate::model::ObservedRegion] { self.regions }
    pub fn source_id(&self) -> u64 { self.owner.value() }
    pub fn horizon_rotation(&self) -> Matrix3 { self.horizon }
    pub fn refraction_enabled(&self) -> bool { self.refraction }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(ObservationRegion { eligible, corrections, apparent });
