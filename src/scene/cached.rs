//! Renderer-owned whole-sky caches. Metadata and terminal presentation are assembled after these immutable results.
use super::{RenderOptions, draw_sky_scene, pixels::draw_pixel_sky};
use crate::{
    cache::{Cache, CacheConfig, Group},
    canvas::Canvas,
    projection::*,
    sky::ObservedStar,
    timing::StepTimes,
};

#[derive(Clone, PartialEq)]
struct SceneKey {
    stars: Vec<(ObservedStar, Option<(i32, i32)>)>,
    planets: Vec<ProjectedPlanet>,
    moon: Option<ProjectedMoon>,
    constellations: Vec<ProjectedConstellation>,
    horizon: Vec<[(i32, i32); 2]>,
    labels: Vec<((i32, i32), &'static str)>,
    viewport: Viewport,
    facing: bool,
    warning: bool,
    options: RenderOptions,
    names: Option<crate::catalog::StarNames>,
}
impl SceneKey {
    fn capture(sky: &ProjectedSky<'_>, options: RenderOptions, characters: bool) -> Self {
        Self {
            stars: sky
                .stars
                .iter()
                .map(|s| {
                    let mut star = s.star.clone();
                    star.position = crate::astro::Vector3::default(); // only screen geometry affects raster output
                    (star, s.cell)
                })
                .collect(),
            planets: sky.planets.clone(),
            moon: sky.moon.cell.map(|_| sky.moon.clone()),
            constellations: sky.constellations.clone(),
            horizon: sky.horizon.clone(),
            labels: sky.horizon_labels.clone(),
            viewport: sky.viewport,
            facing: sky.facing,
            warning: sky.outside_accuracy_range,
            options,
            names: characters.then(|| sky.names.clone()),
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
        let key = SceneKey::capture(sky, *options, false);
        if self
            .pixels
            .needs_refresh(&key, epoch, None, self.config.allows(Group::Raster))
        {
            let image = draw_pixel_sky(sky, options, times)?;
            self.pixels.store(key, epoch, 0.0, image);
        }
        Some(self.pixels.value().clone())
    }
    pub fn draw_characters(
        &mut self,
        canvas: &mut Canvas,
        sky: &ProjectedSky<'_>,
        options: &RenderOptions,
        epoch: f64,
    ) {
        let key = SceneKey::capture(sky, *options, true);
        if self
            .characters
            .needs_refresh(&key, epoch, None, self.config.allows(Group::Raster))
        {
            draw_sky_scene(canvas, options, sky);
            self.characters.store(key, epoch, 0.0, canvas.clone());
        } else {
            canvas.clone_from(self.characters.value());
        }
    }
}
