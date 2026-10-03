//! Renderer-owned whole-sky caches. Metadata and terminal presentation are assembled after these immutable results.
mod keys;
#[cfg(test)]
mod tests;

use super::{RenderOptions, draw_sky_scene_with_times, pixels::draw_pixel_sky};
use crate::{
    cache::{Cache, CacheConfig, Group},
    canvas::Canvas,
    projection::*,
    timing::StepTimes,
};
use keys::StarKeys;

#[derive(Clone, PartialEq)]
struct SceneKey {
    stars: StarKeys,
    planets: Vec<ProjectedPlanet>,
    moon: Option<ProjectedMoon>,
    constellations: Vec<ProjectedConstellation>,
    horizon: Vec<[(i32, i32); 2]>,
    labels: Vec<((i32, i32), &'static str)>,
    viewport: Viewport,
    facing: bool,
    warning: bool,
    options: RenderOptions,
    canvas_size: Option<(usize, usize)>,
}
impl SceneKey {
    fn capture(sky: &ProjectedSky<'_>, options: RenderOptions, canvas_size: Option<(usize, usize)>) -> Self {
        Self {
            stars: StarKeys::capture(sky, &options, canvas_size.is_some()),
            planets: sky.planets.clone(),
            moon: sky.moon.cell.map(|_| sky.moon.clone()),
            constellations: sky.constellations.clone(),
            horizon: sky.horizon.clone(),
            labels: sky.horizon_labels.clone(),
            viewport: sky.viewport,
            facing: sky.facing,
            warning: sky.outside_accuracy_range,
            options,
            canvas_size,
        }
    }
}
#[derive(Default)]
pub struct SceneCache {
    config: CacheConfig,
    pixels: Cache<SceneKey, image::RgbaImage>,
    characters: Cache<SceneKey, Canvas>,
}
impl SceneCache {
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
    pub fn draw_pixels(
        &mut self,
        sky: &ProjectedSky<'_>,
        options: &RenderOptions,
        epoch: f64,
        times: &mut StepTimes,
    ) -> Option<image::RgbaImage> {
        let key = times.measure("Raster cache key", || SceneKey::capture(sky, *options, None));
        times.describe("Raster cache key", || key.stars.describe(sky.stars.len()));
        let refresh = times.measure("Raster cache decision", || {
            self.pixels
                .needs_refresh(&key, epoch, None, self.config.allows(Group::Raster))
        });
        if refresh {
            let image = draw_pixel_sky(sky, options, times)?;
            times.measure("Raster cache store", || self.pixels.store(key, epoch, 0.0, image));
        } else {
            times.measure("Unused raster key release", || drop(key));
        }
        let image = times.measure("Raster output copy", || self.pixels.value().clone());
        times.describe("Raster output copy", || format!("copied RGBA bytes={}", image.len()));
        Some(image)
    }
    pub fn draw_characters(
        &mut self,
        canvas: &mut Canvas,
        sky: &ProjectedSky<'_>,
        options: &RenderOptions,
        epoch: f64,
    ) {
        self.draw_characters_with_times(canvas, sky, options, epoch, &mut StepTimes::default());
    }
    pub(crate) fn draw_characters_with_times(
        &mut self,
        canvas: &mut Canvas,
        sky: &ProjectedSky<'_>,
        options: &RenderOptions,
        epoch: f64,
        times: &mut StepTimes,
    ) {
        let key = times.measure("Raster cache key", || {
            SceneKey::capture(sky, *options, Some((canvas.height(), canvas.width())))
        });
        times.describe("Raster cache key", || key.stars.describe(sky.stars.len()));
        let refresh = times.measure("Raster cache decision", || {
            self.characters
                .needs_refresh(&key, epoch, None, self.config.allows(Group::Raster))
        });
        if refresh {
            draw_sky_scene_with_times(canvas, options, sky, times);
            let copy = times.measure("Character cache copy", || canvas.clone());
            times.measure("Raster cache store", || self.characters.store(key, epoch, 0.0, copy));
        } else {
            times.measure("Raster output copy", || canvas.clone_from(self.characters.value()));
            times.measure("Unused raster key release", || drop(key));
        }
    }
}
