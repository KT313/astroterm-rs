//! Whole-sky raster snapshots. Keys preserve structural equality.
//! Pixel output is borrowed; metadata is composed into a separate full-frame image.
use crate::cache::{Cache, CacheConfig};
use crate::canvas::Canvas;
use crate::model::SceneKey;
#[derive(Default)]
pub struct SceneCache {
    pub(crate) config: CacheConfig,
    /// Premultiplied star pixels in row order. Refilled on redraw; capacity survives frames and resize.
    /// Kept separate from tiny-skia's 8-bit scene; 16 bytes per pixel, reported in memory diagnostics.
    pub(crate) star_layer: Vec<crate::model::StarPixel>,
    /// One opacity per catalog magnitude code for the current zoom boost; rebuilt only when the field of view changes it.
    pub(crate) star_opacities: crate::model::StarOpacityTable,
    /// Regional dependency records for production, exact drawing inputs for editable callers.
    /// Hits clear live entries but retain flat capacities;
    /// successful refreshes transfer the candidate into the matching cache. Failed pixel draws retain it.
    /// Nested label strings and constellation arc payloads are dropped when the candidate is cleared.
    pub(crate) pixel_candidate: Option<SceneKey>,
    pub(crate) pixel_inputs: Vec<crate::model::PixelStarKey>, // filled only for a trusted production redraw
    /// Bytes of the image displaced by the last pixel store; the next redraw draws into this allocation.
    pub(crate) image_scratch: Vec<u8>,
    pub(crate) character_candidate: Option<SceneKey>,
    /// Completed sky pixels before metadata; the renderer borrows this allocation.
    pub(crate) pixels: Cache<SceneKey, image::RgbaImage>,
    /// Completed character sky; refresh and hit copies preserve existing canvas semantics.
    pub(crate) characters: Cache<SceneKey, Canvas>,
}
impl SceneCache {
    /// Advances only when completed pixels differ, including switches between exact and trusted inputs.
    pub fn pixel_generation(&self) -> u64 { self.pixels.generation }

    /// Incomplete or invalidated pixels cannot serve as a completed image dependency.
    pub(crate) fn ready_pixel_generation(&self) -> Option<u64> {
        (!self.pixels.has_been_invalidated && self.pixels.stored().is_some()).then_some(self.pixels.generation)
    }

    /// Read the completed sky; callers must prepare it before borrowing and cannot paint into it.
    pub fn pixel_image(&self) -> &image::RgbaImage { self.pixels.value() }

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
crate::cache::report_fields!(SceneCache { config, star_layer, star_opacities, pixel_inputs, image_scratch, pixel_candidate, character_candidate, pixels, characters });
