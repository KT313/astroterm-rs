//! astroterm: a terminal star map.
//!
//! The frame loop is visible in the binary's `pipeline.rs`: independently prepare simulation samples, observe
//! them from a site, project the apparent sky, then draw and present it through [`terminal::Renderer`].
//!
//! Pure foundations ([`astro`], [`canvas`], [`catalog`], [`cache`] and [`timing`]) support the shared records in
//! [`model`]. Catalog and observed data, view settings, projected geometry and validated configuration live there;
//! representation accessors and constructors do not depend on higher processing modules. [`sky`], [`projection`]
//! and [`scene`] contain preparation, correction, geometric and drawing algorithms over restricted borrowed inputs.
//! [`state::ApplicationState`] owns the catalog, processing caches, rendering buffers and designated scratch after
//! catalog loading. Only the terminal writer/restoration guard stays scoped outside it. The optional
//! `memory-diagnostics` feature inventories those owners without changing their calculations or cache policies.
//!
//! [`astro::models`] keeps independent star, planet, Moon and orientation formulas. [`controls`] changes views and
//! the simulation clock; [`cli`] parses/validates input, and [`terminal`] owns scoped I/O and restoration. Shared data uses canonical `model`/`state` paths;
//! preparation and geometric operations are explicit free functions. Each of these modules exposes its API
//! through `mod.rs`; optional `pipeline.rs` files show ordered processing while private supporting folders hold
//! implementation details. Callers import root symbols instead of depending on those folders.
//!
//! Earth is the only production anchor. Common f64 states use equatorial J2000 axes, AU and AU/day, with
//! a barycentric origin. [`sky`] documents the observer site and apparent-place corrections.

pub mod astro;
pub mod cache;
pub mod canvas;
pub mod catalog;
pub mod cli;
pub mod controls;
pub mod metadata;
pub mod model;
pub mod projection;
pub mod scene;
pub mod sky;
pub mod state;
pub mod terminal;
pub mod timing;
