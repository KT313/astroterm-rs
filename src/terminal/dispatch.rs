//! Scoped terminal guard and rendering algorithms over application-owned buffers.
use crate::model::config::TerminalSettings;
use super::{TerminalSession, open_terminal_renderer, pixels};
use crate::astro::{Observer, SimulationClock};
use crate::model::projection::{ProjectedSky, ProjectionViewport as Viewport, View};
use crate::model::rendering::RenderOptions;
use crate::state::rendering::RenderingState;
use crate::timing::StepTimes;
use ratatui_image::picker::ProtocolType;
use std::io;

use crate::model::config::{GraphicsProtocol, RendererKind};

fn select_forced_protocol(value: GraphicsProtocol) -> Option<ProtocolType> {
    match value {
        GraphicsProtocol::Auto => None,
        GraphicsProtocol::Kitty => Some(ProtocolType::Kitty),
        GraphicsProtocol::Sixel => Some(ProtocolType::Sixel),
        GraphicsProtocol::Iterm2 => Some(ProtocolType::Iterm2),
        GraphicsProtocol::Halfblocks => Some(ProtocolType::Halfblocks),
    }
}

/// Only the output writer and terminal-restoration lifetime. Application buffers belong to RenderingState.
pub struct Renderer { session: TerminalSession }
impl Renderer {
    pub fn configure_cache(&mut self, state: &mut RenderingState, config: &crate::cache::CacheConfig) {
        match state {
            RenderingState::Chars(r) => r.scene_cache.configure(config),
            RenderingState::Pixels(r) => {
                r.scene_cache.configure(config);
                r.reuse_assets = config.allows(crate::cache::Group::RasterAssets);
            }
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    pub fn prepare_catalog(&mut self, state: &mut RenderingState, catalog: std::sync::Arc<crate::model::SkyCatalog>, times: &mut StepTimes) {
        match state {
            RenderingState::Chars(r) => crate::scene::cached::prepare_scene_catalog(&mut r.scene_cache, catalog, times),
            RenderingState::Pixels(r) => crate::scene::cached::prepare_scene_catalog(&mut r.scene_cache, catalog, times),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    pub fn set_cache_diagnostics(&mut self, state: &mut RenderingState, observation: crate::cache::CacheStats, projection: crate::cache::CacheStats) {
        let value = [crate::cache::format_stats(observation), crate::cache::format_stats(projection)];
        match state {
            RenderingState::Chars(r) => r.cache_diagnostics = value,
            RenderingState::Pixels(r) => r.cache_diagnostics = value,
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    pub fn open(kind: RendererKind, protocol: GraphicsProtocol, options: RenderOptions, settings: TerminalSettings, text_scale: f64) -> io::Result<(Self, RenderingState)> {
        if kind == RendererKind::Pixels {
            match pixels::open_pixel_renderer(options, settings, select_forced_protocol(protocol), text_scale) {
                Ok((session, state)) => return Ok((Self { session }, RenderingState::Pixels(Box::new(state)))),
                Err(error) => {
                    let (session, mut state) = open_terminal_renderer(options, settings)?;
                    state.startup_notice = Some(format!("Pixel startup failed; using characters: {error}"));
                    return Ok((Self { session }, RenderingState::Chars(Box::new(state))));
                }
            }
        }
        let (session, state) = open_terminal_renderer(options, settings)?;
        Ok((Self { session }, RenderingState::Chars(Box::new(state))))
    }
    pub fn viewport(&self, state: &RenderingState) -> Viewport {
        match state {
            RenderingState::Chars(state) => super::renderer::character_viewport(state),
            RenderingState::Pixels(state) => pixels::pixel_viewport(state),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    pub fn fit_to_terminal(&mut self, state: &mut RenderingState) -> io::Result<()> {
        match state {
            RenderingState::Chars(state) => super::renderer::fit_character_terminal(state, &mut self.session),
            RenderingState::Pixels(state) => pixels::fit_pixel_terminal(state, &mut self.session),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render_frame(&mut self, state: &mut RenderingState, sky: &ProjectedSky<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<()> {
        match state {
            RenderingState::Chars(state) => super::renderer::render_character_frame(state, &mut self.session, sky, view, date, clock, observer, times),
            RenderingState::Pixels(state) => pixels::render_pixel_frame(state, &mut self.session, sky, view, date, clock, observer, times),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::buffers::report_fields!(Renderer { session });
