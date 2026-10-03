//! astroterm: a terminal star map.
//!
//! The frame loop is visible in `main.rs`: independently refresh [`sky::SimulationState`], observe it at the current
//! epoch and site into [`sky::ObservedSky`], project it with [`projection::project_sky`], then draw prepared screen
//! geometry with [`scene`]. [`terminal::Renderer`] selects character diffing or pixel image/text presentation.
//!
//! [`astro::models`] separates stars, planets, moons and orientation, including their coefficients and reference
//! tests. Shared time, coordinate and orbital math remain in [`astro`]; model code never imports catalog I/O,
//! observers or renderers. [`catalog`] parses inputs; [`sky::SkyCatalog`] owns immutable star data. [`controls`]
//! changes views and the simulation clock without invalidating geometric caches. [`timing`] records stage costs.
//!
//! Earth is the only production anchor. Common f64 states use equatorial J2000 axes, AU and AU/day, with
//! a barycentric origin. [`sky`] documents the observer site and apparent-place corrections.

pub mod astro;
pub mod canvas;
pub mod catalog;
pub mod cli;
pub mod controls;
pub mod metadata;
pub mod projection;
pub mod scene;
pub mod sky;
pub mod terminal;
pub mod timing;
