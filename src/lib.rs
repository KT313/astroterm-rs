//! astroterm: a terminal star map.
//!
//! The binary's `pipeline.rs` calls seven frame steps in order: [`solar_system`], [`observation`], [`selection`],
//! [`regions`], [`projection`], [`points`] and [`present`]. [`sky`] loads and prepares catalogs before that loop.
//!
//! [`model::Data`] owns the solar-system tables, constellation stars and figures, shared names, and ordinary
//! sky regions. Each region owns its speed groups and merged tile; each group owns its original star columns,
//! calculated samples and rendered tile. Definitions are collected in `model/data.rs`; processing algorithms
//! remain in the step modules. Catalog columns stay unchanged during frames, while their caches update in place.
//!
//! Pure foundations ([`formulas`], [`catalog`], [`rows`], [`progress`], [`time`] and [`text`]) support the model.
//! [`formulas`] supplies astronomy formulas. [`controls`] changes the view and simulation clock; [`cli`]
//! validates options; [`terminal`] handles input, transport and terminal restoration. Callers import public
//! symbols from each module's root, not its implementation folders.
//!
//! Earth is the production observer anchor. Positions use equatorial J2000 axes, AU and AU/day, with a
//! barycentric origin. The frame buffer and terminal session have their own lifetimes outside Data.

/// Tunable defaults, resource limits and cache durations; independent of all application layers.
pub mod constants;
#[cfg(feature = "pixel-debug")]
pub mod pixel_debug;
/// Standalone physical formulas and shared mathematical types; no imports from other application modules.
pub mod formulas;
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
pub mod selection;
pub mod solar_system;
pub mod sky;
pub mod terminal;
pub mod text;
/// Self-contained simulation clocks and timing; application settings are supplied by callers.
pub mod time;
