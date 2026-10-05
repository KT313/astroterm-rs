//! Catalog-indexed selection and independent correction snapshots. No corrected direction is model input.
//! Each working record carries a catalog source_index; working indices address working/motion/eligibility arrays.
//! Correction indices select working records; the resulting observed array has its own indices. Generations link passes.
use crate::astro::{Matrix3, Vector3, models::{BodyState, stars::StellarSample}};
use crate::cache::{Cache, CacheConfig};
use crate::model::SkyCatalog;
use crate::model::observation::{ObserverKey, ObserverState, Directions, BodyKey, SelectedStar, BodySamples, CorrectionSelection};
use std::{collections::HashMap, sync::Arc};
pub(crate) type RegionCache = Cache<(crate::model::SkyRegion, ObserverState, bool), crate::model::grid::SelectedRegion>;
pub(crate) type CandidateCache = Cache<(u64, f64), (Vec<usize>, crate::model::SelectionStats)>;
pub(crate) type SelectedCache = Cache<(u64, bool), Vec<usize>>;
pub(crate) type WorkingCache = Cache<u64, Vec<SelectedStar>>;
pub(crate) type MotionCache = Cache<u64, (Vec<(Vector3, f64)>, usize)>;
pub(crate) type EligibleCache = Cache<(u64, u64, f64), Vec<bool>>;
pub(crate) type RelativeCache = Cache<(u64, BodyState), (Vec<Vector3>, Vector3)>;
pub(crate) type IlluminationCache = Cache<(Vector3, Vector3), (crate::model::MoonIllumination, crate::astro::MoonPhase)>;
pub(crate) type ApparentCache = Cache<(u64, u64, u64, Vector3), Directions>;
pub(crate) type HorizontalCache = Cache<(u64, Matrix3), Directions>;

#[derive(Default)]
pub struct ObservationCache {
    pub(crate) config: CacheConfig,
    pub(crate) catalog: Option<Arc<SkyCatalog>>,                         // retained identity; catalog replacement clears all dependent fields
    pub(crate) prepared_classes: Option<Vec<crate::astro::models::stars::StellarClass>>, // immutable catalog-row classifications; absent means classify on demand
    pub(crate) observer: Cache<ObserverKey, ObserverState>,             // reception site/rotation in TT; independent of star catalog
    pub(crate) light_time: Cache<(ObserverState, [u64; 3]), ObserverState>, // emission times; independent of star catalog
    pub(crate) region: RegionCache,                                    // conservative grid membership from camera/observer/refraction
    pub(crate) candidates: CandidateCache,                             // catalog row indices surviving brightness bounds, plus counts
    pub(crate) selected: SelectedCache,                                // validated catalog indices; endpoint merge sorts/deduplicates these
    pub(crate) working: WorkingCache,                                  // sorted catalog indices and drawable flags, including constellation endpoints
    pub(crate) stellar: HashMap<usize, Cache<(), StellarSample>>,       // intrinsic J2000 direction/magnitude per catalog row; no eviction
    pub(crate) stellar_scratch: Vec<crate::model::observation::StellarWork>, // at most 1024 live records; clear after refresh, retain capacity
    pub(crate) stellar_stats: crate::cache::CacheStats,
    pub(crate) motion: MotionCache,                                    // J2000 unit directions/current magnitudes in working order, plus singular count
    pub(crate) eligible: EligibleCache,                                // current drawing eligibility in working order
    pub(crate) corrections: Cache<(u64, u64), CorrectionSelection>,     // working indices retained for drawing or constellation geometry
    pub(crate) bodies: Cache<BodyKey, BodySamples>,                     // barycentric AU states sampled at emission times
    pub(crate) relative: RelativeCache,                                // observer-relative AU vectors in planet order, plus Moon
    pub(crate) illumination: IlluminationCache,                        // Moon lighting from relative Sun/Moon geometry
    pub(crate) apparent: ApparentCache,                                // independent aberrated directions in corrected-star/body order
    pub(crate) horizontal: HorizontalCache,                            // independent East/North/Up directions in the same order
    pub(crate) refracted: Cache<(u64, bool), Directions>,                // independent refracted horizontal directions; never fed back into motion
}

/// Only stellar sample/cache outputs can change; membership and catalog classifications stay borrowed.
pub(crate) struct StellarMotionBuffers<'a> {
    pub config: &'a CacheConfig,
    pub working: &'a WorkingCache,
    pub prepared_classes: Option<&'a [crate::astro::models::stars::StellarClass]>,
    pub stellar: &'a mut HashMap<usize, Cache<(), StellarSample>>,
    pub scratch: &'a mut Vec<crate::model::observation::StellarWork>,
    pub stats: &'a mut crate::cache::CacheStats,
    pub motion: &'a mut MotionCache,
}

/// Observer preparation can mutate only its reception cache, never selection/correction buffers.
pub(crate) struct ObserverBuffers<'a> {
    pub config: &'a CacheConfig,
    pub observer: &'a mut Cache<ObserverKey, ObserverState>,
}

/// Emission preparation can mutate only its result cache; simulation owns the actual body samples.
pub(crate) struct LightTimeBuffers<'a> {
    pub config: &'a CacheConfig,
    pub light_time: &'a mut Cache<(ObserverState, [u64; 3]), ObserverState>,
}
impl ObservationCache {
    pub(crate) fn borrow_stellar_motion(&mut self) -> StellarMotionBuffers<'_> {
        StellarMotionBuffers {
            config: &self.config, working: &self.working, prepared_classes: self.prepared_classes.as_deref(),
            stellar: &mut self.stellar, scratch: &mut self.stellar_scratch, stats: &mut self.stellar_stats,
            motion: &mut self.motion,
        }
    }
    pub(crate) fn borrow_observer(&mut self) -> ObserverBuffers<'_> {
        ObserverBuffers { config: &self.config, observer: &mut self.observer }
    }
    pub(crate) fn borrow_light_time(&mut self) -> LightTimeBuffers<'_> {
        LightTimeBuffers { config: &self.config, light_time: &mut self.light_time }
    }

    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn invalidate_view(&mut self) {
        self.region.invalidate();
    }
    pub fn observer_report(&self) -> crate::cache::CacheReport { self.observer.report("Observer geometry") }
    pub fn light_time_report(&self) -> crate::cache::CacheReport { self.light_time.report("Light-time sampling") }

    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![
            self.observer_report(),
            self.light_time_report(),
            self.region.report("Region filtering"),
            self.candidates.report("Brightness bounds"),
            self.selected.report("Candidate validation"),
            self.working.report("Constellation endpoints"),
            self.motion.report("Stellar motion"),
            self.eligible.report("Current brightness"),
            self.corrections.report("Correction selection"),
            self.bodies.report("Body sampling"),
            self.relative.report("Observer subtraction"),
            self.illumination.report("Moon illumination"),
            self.apparent.report("Aberration"),
            self.horizontal.report("Horizon rotation"),
            self.refracted.report("Refraction"),
        ]
    }
    pub fn stellar_report(&self, index: usize) -> Option<crate::cache::CacheReport> {
        self.stellar.get(&index).map(|c| c.report("Stellar state"))
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.observer.stats,
            self.light_time.stats,
            self.region.stats,
            self.candidates.stats,
            self.selected.stats,
            self.working.stats,
            self.motion.stats,
            self.eligible.stats,
            self.corrections.stats,
            self.bodies.stats,
            self.relative.stats,
            self.illumination.stats,
            self.apparent.stats,
            self.horizontal.stats,
            self.refracted.stats,
        ]
        .into_iter()
        .chain([self.stellar_stats])
        {
            total.hits += s.hits;
            total.refreshes += s.refreshes;
            total.bypasses += s.bypasses;
        }
        total
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::buffers::report_fields!(ObservationCache { config, catalog, prepared_classes, observer, light_time, region, candidates, selected, working, stellar, stellar_scratch, motion, eligible, corrections, bodies, relative, illumination, apparent, horizontal, refracted });
