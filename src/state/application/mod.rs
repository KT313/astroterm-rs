//! Construct the complete root from validated configuration; the prepared catalog is installed afterwards.
//!
//! Root groups follow their lifetimes; preparation is freed before the frame loop:
//! - `current_view`: the mutable camera; config.view remains the initial reset target,
//! - `persistent`: loaded once at startup and never changed during a run (the prepared catalog),
//! - `preparation`: temporary catalog bounds used only while building/validating the catalog,
//! - `cache`: everything recomputed from the persistent data and the simulated time (observed objects, caches, buffers),
//! - `timings`: diagnostics, not used for rendering.
mod caches;
pub use caches::Caches;

use std::sync::Arc;
use crate::model::{Config, View, Sky, SkyCatalog, CatalogPreparation, PreparedCatalog};
use crate::timing::StepTimes;

pub struct ApplicationState {
    pub config: Config,
    /// Camera changed by pan/zoom controls; config.view retains the original reset value.
    pub current_view: View,
    pub persistent: Persistent,
    pub preparation: Option<CatalogPreparation>,
    pub cache: Caches,
    pub timings: StepTimes,
}

/// Data loaded once and shared read-only by every stage for the whole run.
pub struct Persistent {
    pub catalog: Arc<SkyCatalog>,
}

impl ApplicationState {
    /// Construct once from validated configuration. Every owner exists with its final type; the catalog starts
    /// empty and `replace_catalog` installs the loaded one before the frame loop.
    pub fn new(config: Config, timings: StepTimes) -> Self {
        let catalog = Arc::new(SkyCatalog::empty());
        let cache = Caches::new(Sky::new(catalog.clone()), &config.cache);
        Self { current_view: config.view, config, persistent: Persistent { catalog }, preparation: None, cache, timings }
    }

    /// Free the exclusively owned startup payload; shared runtime inputs keep their allocation and identity.
    pub fn free_preparation_only_data(&mut self) { self.preparation = None; }

    pub fn preparation(&self) -> Option<&CatalogPreparation> { self.preparation.as_ref() }

    /// Install the prepared catalog: the only catalog mutation, done once at startup. The observed sky is rebuilt
    /// from shared inputs; dependent catalog caches are reset and `prepare_frame_data` runs later.
    /// The same `Arc` is shared by `persistent.catalog` and `cache.sky.catalog`, so no catalog buffers are copied.
    pub fn replace_catalog(&mut self, prepared: PreparedCatalog) {
        let catalog = Arc::new(prepared.catalog);
        self.preparation = Some(prepared.preparation);
        self.persistent.catalog = catalog.clone();
        self.cache.sky = Sky::new(catalog);
        self.cache.selection = crate::state::StarSelectionCache::new(self.config.cache.clone());
        self.cache.simulation.stars = crate::state::StellarSimulationState::new(self.config.cache.clone());
        self.cache.observer = crate::state::ObserverPreparationCache::new(self.config.cache.clone());
        self.cache.observation = crate::state::ObservationCache::new(self.config.cache.clone());
        self.cache.projection = crate::state::ProjectionCache::new(self.config.cache.clone());
        let scene = match &mut self.cache.rendering {
            crate::state::RenderingState::Pending => return,
            crate::state::RenderingState::Chars(state) => &mut state.scene_cache,
            crate::state::RenderingState::Pixels(state) => &mut state.scene_cache,
        };
        *scene = crate::state::SceneCache::default();
        scene.configure(&self.config.cache);
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Persistent { catalog });
