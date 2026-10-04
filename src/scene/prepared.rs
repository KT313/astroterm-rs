//! Catalog-bound display constants prepared once before the frame loop. A strong owner prevents address reuse;
//! callers rendering another catalog use the original formulas instead of reading stale constants.
use crate::{
    canvas::Color,
    sky::{ObservedStarView, SkyCatalog},
    timing::StepTimes,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct StarDisplay {
    rgb: [u8; 3],
    color: Option<Color>,
    named: bool,
}

pub(crate) struct PreparedScene {
    catalog: Arc<SkyCatalog>,
    stars: Vec<StarDisplay>,
}

impl PreparedScene {
    pub fn new(catalog: Arc<SkyCatalog>, times: &mut StepTimes) -> Self {
        let stars: Vec<_> = times.measure("Star display constants", || {
            (0..catalog.stars.len())
                .map(|i| {
                    let spectral = catalog.stars.spectral_type(i);
                    let color_index = catalog.stars.color_index(i);
                    StarDisplay {
                        rgb: super::pixels::compute_star_rgb(spectral, color_index),
                        color: super::appearance::select_star_color(spectral, color_index),
                        named: catalog.stars.name(i).is_some(),
                    }
                })
                .collect()
        });
        times.describe("Star display constants", || format!("stars={}; named candidates={}; prepared RGB/character color/name-eligibility bytes={}; brightness-dependent rendering remains dynamic", stars.len(), stars.iter().filter(|s| s.named).count(), stars.len()*std::mem::size_of::<StarDisplay>()));
        Self { catalog, stars }
    }

    fn find(&self, star: &ObservedStarView<'_>) -> Option<&StarDisplay> {
        std::ptr::eq(star.catalog, &self.catalog.stars).then(|| &self.stars[star.source_index])
    }
    pub fn rgb(&self, star: &ObservedStarView<'_>) -> [u8; 3] {
        self.find(star).map_or_else(|| super::pixels::star_rgb(star), |s| s.rgb)
    }
    pub fn color(&self, star: &ObservedStarView<'_>) -> Option<Color> {
        self.find(star).map_or_else(
            || super::appearance::select_star_color(star.spectral_type(), star.color_index()),
            |s| s.color,
        )
    }
    pub fn is_named(&self, star: &ObservedStarView<'_>) -> bool {
        self.find(star).map_or_else(|| star.name().is_some(), |s| s.named)
    }
}

pub(crate) fn resolve_star_rgb(star: &ObservedStarView<'_>, prepared: Option<&PreparedScene>) -> [u8; 3] {
    prepared.map_or_else(|| super::pixels::star_rgb(star), |p| p.rgb(star))
}
