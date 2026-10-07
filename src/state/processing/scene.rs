//! Whole-sky raster snapshots and catalog-derived display data. Keys preserve structural equality.
//! Pixel output is borrowed; metadata is composed into a separate full-frame image.
use crate::cache::{Cache, CacheConfig};
use crate::canvas::Canvas;
use crate::model::{PreparedScene, SceneKey};
#[derive(Default)]
pub struct SceneCache {
    pub(crate) config: CacheConfig,
    /// Immutable catalog display constants; replace at catalog preparation, shared catalog payload stays shared.
    pub(crate) prepared: Option<PreparedScene>,
    /// Indices into the current projected draw order for stars with names; rebuilt during pixel key capture.
    pub(crate) named_candidates: Vec<usize>,
    /// Exact raster inputs in draw order. Hits clear live entries but retain flat capacities;
    /// successful refreshes transfer the candidate into the matching cache. Failed pixel draws retain it.
    /// Nested label strings and constellation arc payloads are dropped when the candidate is cleared.
    pub(crate) pixel_candidate: Option<SceneKey>,
    pub(crate) character_candidate: Option<SceneKey>,
    /// Completed sky pixels before metadata; the renderer borrows this allocation.
    pub(crate) pixels: Cache<SceneKey, image::RgbaImage>,
    /// Completed character sky; refresh and hit copies preserve existing canvas semantics.
    pub(crate) characters: Cache<SceneKey, Canvas>,
}
impl SceneCache {
    /// Read the completed sky; callers must prepare it before borrowing and cannot paint into it.
    pub fn pixel_image(&self) -> &image::RgbaImage { self.pixels.value() }

    pub(crate) fn prepared(&self) -> Option<&PreparedScene> {
        self.prepared.as_ref()
    }
    pub(crate) fn named_candidates(&self) -> Option<&[usize]> {
        self.prepared.as_ref().map(|_| self.named_candidates.as_slice())
    }
    pub fn configure(&mut self, config: &CacheConfig) {
        self.config = config.clone();
        self.invalidate();
    }
    pub fn invalidate(&mut self) {
        self.pixels.invalidate();
        self.characters.invalidate();
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let a = self.pixels.stats;
        let b = self.characters.stats;
        crate::cache::CacheStats {
            hits: a.hits + b.hits,
            refreshes: a.refreshes + b.refreshes,
            bypasses: a.bypasses + b.bypasses,
            last_reason: a.last_reason.or(b.last_reason),
        }
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(SceneCache { config, prepared, named_candidates, pixel_candidate, character_candidate, pixels, characters });
