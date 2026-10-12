//! astroterm: a terminal star map.
//!
//! The frame loop is visible in the binary's `pipeline.rs`: simulate the solar system, prepare observer/light-time
//! inputs, select stars, simulate those stars, apply observation corrections, project, then render through
//! [`terminal::Renderer`]. Simulation and selection have separate owners and exchange read-only borrowed views.
//!
//! Pure foundations ([`astro`], [`canvas`], [`catalog`], [`cache`], [`rows`], [`progress`] and [`timing`]) support the shared records in
//! [`model`]. Catalog and observed data, view settings, projected geometry and validated configuration live there;
//! representation accessors and constructors do not depend on higher processing modules. [`sky`], [`projection`]
//! and [`scene`] contain preparation, correction, geometric and drawing algorithms over restricted borrowed inputs.
//! [`state::ApplicationState`] owns the catalog, processing caches, rendering buffers and designated scratch from
//! startup; the prepared catalog is installed through `replace_catalog`. Only the terminal writer/restoration guard
//! stays scoped outside it.
//!
//! [`astro::models`] keeps independent star, planet, Moon and orientation formulas. [`controls`] changes views and
//! the simulation clock; [`cli`] parses/validates input, and [`terminal`] owns scoped I/O and restoration. Shared data uses canonical `model`/`state` paths;
//! preparation and geometric operations are explicit free functions. Each of these modules exposes its API
//! through `mod.rs`; optional `pipeline.rs` files show ordered processing while private supporting folders hold
//! implementation details. Callers import root symbols instead of depending on those folders.
//!
//! The frame pipeline is being rebuilt (`dev/pipeline-restructuring-plan.md`). The new spine grows one step module
//! at a time directly under `src/`: [`solar_system`] (step 1), [`observation`] (step 2), [`selection`] (step 3) and
//! [`regions`] (step 4: speed groups, simulation caches, sky-fixed group and region images, `refresh_regions`) and
//! [`projection`] (step 5, `project_stars_to_screen`: the warp of the region tiles onto the screen, beside the
//! projection geometry it reuses), [`points`] (step 6, `draw_points`: bodies, constellation lines, labels and
//! overlays on the frame) and [`present`] (step 7, `present`: tone mapping and the terminal output).
//!
//! Earth is the only production anchor. Common f64 states use equatorial J2000 axes, AU and AU/day, with
//! a barycentric origin. [`sky`] documents the observer site and apparent-place corrections.

/// Tunable defaults, resource limits and cache durations; independent of all application layers.
pub mod constants;
pub mod astro;
pub mod cache;
pub mod canvas;
pub mod catalog;
pub mod cli;
pub mod controls;
pub mod metadata;
pub mod model;
pub mod observation;
pub mod points;
pub mod present;
pub mod projection;
pub mod regions;
pub mod progress;
pub mod rows;
pub mod scene;
pub mod selection;
pub mod solar_system;
pub mod sky;
pub mod state;
pub mod terminal;
pub mod timing;

#[cfg(test)]
extern crate self as astroterm;
#[cfg(test)]
#[path = "../tests/support/cached.rs"]
pub(crate) mod test_pipeline;
