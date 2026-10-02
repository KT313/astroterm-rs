//! astroterm: a terminal star map.
//!
//! The crate is layered bottom-up: [`astro`] (time, coordinates, ephemeris) and [`canvas`] (cell grid, line drawing)
//! are the foundations; [`projection`] and [`catalog`] build on them; [`sky`] holds the object model; [`scene`] draws
//! the sky onto a canvas; [`terminal`], [`controls`] and [`cli`] connect it to the user.

pub mod astro;
pub mod canvas;
pub mod catalog;
pub mod cli;
pub mod controls;
pub mod projection;
pub mod scene;
pub mod sky;
pub mod terminal;
