//! Correction caches over prepared read-only model results. No intrinsic stellar motion is stored here.
//! Regional flags and correction records are authoritative; only small request metadata spans regions.
//! Separate coordinate-space snapshots prevent a corrected direction from becoming a model input.
use crate::astro::{Matrix3, Vector3, models::BodyState};
use crate::cache::{Cache, CacheConfig};
use crate::model::{SkyCatalog, Directions, ObservationRegion};
use std::sync::Arc;
pub(crate) type RelativeCache = Cache<(u64, BodyState), (Vec<Vector3>, Vector3)>;
pub(crate) type IlluminationCache = Cache<(Vector3, Vector3), (crate::model::MoonIllumination, crate::astro::MoonPhase)>;
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
    pub(crate) horizontal_sources: HorizontalSources,                  // region spans/versions and body version used by the horizontal cache
    pub(crate) relative: RelativeCache,                                // observer-relative AU vectors in planet order, plus Moon
    pub(crate) illumination: IlluminationCache,                        // Moon lighting from relative Sun/Moon geometry
    pub(crate) layout_sources: Vec<(usize, u64, u64)>,
    pub(crate) layout_stats: crate::model::CorrectionStats,
    pub(crate) horizontal_work: Directions,
    pub(crate) refraction_work: Directions,
    pub(crate) published: Option<crate::state::StellarPublication>,
    pub(crate) use_refraction: bool,
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
        self.published = None;
    }

    /// Whole-frame caches only; brightness and correction selection have regional reports.
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![
            self.relative.report("Observer subtraction"),
            self.illumination.report("Moon illumination"),
            self.body_apparent.report("Body aberration"),
            self.horizontal.report("Horizon rotation"),
            self.refracted.report("Refraction"),
        ]
    }

    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.relative.stats,
            self.illumination.stats,
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
        crate::cache::report_field(sink, "horizontal_sources", &self.horizontal_sources);
        crate::cache::report_field(sink, "layout_sources", &self.layout_sources);
        crate::cache::report_field(sink, "horizontal_work", &self.horizontal_work);
        crate::cache::report_field(sink, "refraction_work", &self.refraction_work);
        crate::cache::report_field(sink, "config", &self.config);
        crate::cache::report_field(sink, "catalog", &self.catalog);
        crate::cache::report_field(sink, "relative", &self.relative);
        crate::cache::report_field(sink, "illumination", &self.illumination);
        crate::cache::report_field(sink, "horizontal", &self.horizontal);
        crate::cache::report_field(sink, "refracted", &self.refracted);
    }
}

/// Read-only handoff constructed by observation. Holding it prevents mutation of its sky and region versions.
#[derive(Clone, Copy)]
pub struct RegionalObservation<'a> {
    pub(crate) sky: crate::model::ObservedSkyView<'a>,
    pub(crate) regions: &'a [crate::model::ObservedRegion],
    pub(crate) owner: super::StageId,
    pub(crate) horizon: Matrix3,
    pub(crate) refraction: bool,
}
impl<'a> RegionalObservation<'a> {
    pub fn sky(&self) -> crate::model::ObservedSkyView<'a> { self.sky }
    pub fn regions(&self) -> &[crate::model::ObservedRegion] { self.regions }
    pub fn source_id(&self) -> u64 { self.owner.value() }
    pub fn horizon_rotation(&self) -> Matrix3 { self.horizon }
    pub fn refraction_enabled(&self) -> bool { self.refraction }
}


/// Dependency metadata only; no stellar or planetary directions are duplicated here.
#[derive(Default)]
pub(crate) struct HorizontalSources {
    pub regions: Vec<(usize, usize, usize, u64, u64)>, // region ID, output start/end, membership version, apparent version
    pub body_generation: Option<u64>,
    pub revision: u64,
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(HorizontalSources { regions });

/// Read-only apparent directions for a completed observation request. Never stored beside its owners.
#[derive(Clone, Copy)]
pub struct ApparentDirections<'a> {
    descriptors: &'a [crate::model::ObservedRegion],
    regions: &'a [ObservationRegion],
    bodies: &'a BodyApparentCache,
}
impl<'a> ApparentDirections<'a> {
    pub(crate) fn new(descriptors: &'a [crate::model::ObservedRegion], regions: &'a [ObservationRegion], bodies: &'a BodyApparentCache) -> Self {
        bodies.value(); // construction follows successful regional and body preparation
        Self { descriptors, regions, bodies }
    }
    /// Each region is checked once before its original direction slice is consumed.
    pub fn regions(&self) -> impl Iterator<Item = (&crate::model::ObservedRegion, &[Vector3])> {
        self.descriptors.iter().map(|descriptor| {
            let region = &self.regions[descriptor.region];
            assert_eq!(region.corrections.generation, descriptor.selection_generation, "apparent membership does not match output");
            assert_eq!(region.apparent.generation, descriptor.apparent_generation, "apparent version does not match output");
            assert!(region.apparent.key().is_some_and(|key| key.0 == descriptor.selection_generation), "apparent directions use older membership");
            let directions = region.apparent.value();
            assert_eq!(directions.len(), descriptor.end - descriptor.start, "apparent region length does not match output");
            (descriptor, directions.as_slice())
        })
    }
    pub fn bodies(&self) -> (&[Vector3], Vector3) {
        let bodies = self.bodies.value();
        (&bodies.0, bodies.1)
    }
    pub(crate) fn star_count(&self) -> usize { self.descriptors.last().map_or(0, |region| region.end) }
    pub(crate) fn body_generation(&self) -> u64 { self.bodies.generation }
    pub(crate) fn dependencies(&self) -> impl ExactSizeIterator<Item = (usize, usize, usize, u64, u64)> {
        self.descriptors.iter().map(|region| (region.region, region.start, region.end, region.selection_generation, region.apparent_generation))
    }
}

impl ObservationCache {
    /// Reborrow completed observation with the same protected regional provenance used by projection.
    pub fn regional_view<'a>(&'a self, stars: crate::state::StellarResults<'a>, summary: &'a crate::model::ObservedSky) -> RegionalObservation<'a> {
        RegionalObservation { sky: self.observed_view(stars, summary), regions: &self.regional_output, owner: self.identity,
            horizon: self.horizontal.key().expect("completed horizontal result").1, refraction: self.use_refraction }
    }

    pub fn observed_view<'a>(&'a self, stars: crate::state::StellarResults<'a>, summary: &'a crate::model::ObservedSky) -> crate::model::ObservedSkyView<'a> {
        assert_eq!(self.published, Some(stars.publication_key()), "observed output does not match stellar request");
        assert!(Arc::ptr_eq(stars.selection.catalog, &summary.catalog) && self.catalog.as_ref().is_some_and(|catalog| Arc::ptr_eq(catalog, &summary.catalog)), "observed catalog does not match");
        let directions = if self.use_refraction { self.refracted.value() } else { self.horizontal.value() };
        let rows = crate::model::ObservedStars::regional(&summary.catalog.stars, &self.regional_output, &self.regions,
            &stars.regions.entries, &summary.catalog.grid.offsets, &directions.0);
        crate::model::ObservedSkyView::cached(summary, rows, directions)
    }
}
