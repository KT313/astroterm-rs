//! astroterm: a terminal star map.
//!
//! The crate is layered bottom-up, one folder per group: [`astro`] (time, coordinates, ephemeris) and [`canvas`]
//! (cell grid, line drawing) are the foundations; [`projection`] and [`catalog`] build on them; [`sky`] holds the
//! object model and updates its positions; [`controls`] (user actions) and [`metadata`] (panel content) are shared by
//! all renderers; [`scene`] draws the sky onto a canvas; [`terminal`] and [`cli`] connect it to the user.
//!
//! Computing the sky and rendering it are separate: [`sky`] only knows what the objects are and where they are, and a
//! renderer reads it each frame to show it. The one renderer so far is [`terminal::TerminalRenderer`], which draws
//! characters with [`scene`]; glyphs, labels and colors are its choice ([`scene::Appearance`]). Input is backend
//! specific too: the terminal maps its keys to [`controls::Control`]s.

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
