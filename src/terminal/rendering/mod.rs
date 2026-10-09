//! Scoped terminal guard and rendering algorithms over application-owned buffers.
use crate::model::TerminalSettings;
mod characters;
mod pixels;

use super::TerminalSession;
use super::pipeline::{render_character_frame, render_pixel_frame};
pub(super) use characters::{prepare_character_timezone, prepare_character_timing_fields, rasterize_character_sky, draw_character_notice, draw_character_panel, present_character_frame};
pub(super) use pixels::{prepare_pixel_timezone, initialize_pixel_canvas, rasterize_pixel_sky, compose_pixel_sky,
    prepare_pixel_fields, layout_pixel_text, prepare_pixel_glyphs, paint_pixel_text, encode_pixel_cells,
    serialize_pixel_cells, present_pixel_cells, prepare_kitty_pixels,
    present_kitty_image};
pub use characters::open_terminal_renderer;
use crate::astro::{Observer, SimulationClock};
use crate::model::{ProjectedSky, ProjectionViewport as Viewport, View, RenderOptions};
use crate::state::RenderingState;
use crate::timing::StepTimes;
use ratatui_image::picker::ProtocolType;
use std::io;

use crate::model::{GraphicsProtocol, RendererKind};

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
                r.display_valid = false;
                r.scene_cache.configure(config);
                r.rgb_version.invalidate();
                r.frame_key = None;
                r.encoding_key = None;
                r.reuse_assets = config.allows(crate::cache::Group::RasterAssets);
            }
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
            RenderingState::Chars(state) => characters::character_viewport(state),
            RenderingState::Pixels(state) => pixels::pixel_viewport(state),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    pub fn fit_to_terminal(&mut self, state: &mut RenderingState) -> io::Result<()> {
        match state {
            RenderingState::Chars(state) => characters::fit_character_terminal(state, &mut self.session),
            RenderingState::Pixels(state) => pixels::fit_pixel_terminal(state, &mut self.session),
            RenderingState::Pending => panic!("rendering state is not initialized"),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render_prepared_frame(&mut self, state: &mut RenderingState, projected: &crate::model::RenderProjection<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<crate::model::RenderOutcome> {
        if let RenderingState::Pixels(state) = state {
            let outcome = render_pixel_frame(state, &mut self.session, projected.sky(), Some(projected), view, date, clock, observer, times)?;
            times.record_frame_presentation(outcome.was_presented());
            return Ok(outcome);
        }
        self.render_frame(state, projected.sky(), view, date, clock, observer, times)?;
        Ok(crate::model::RenderOutcome::Presented)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render_frame(&mut self, state: &mut RenderingState, sky: &ProjectedSky<'_>, view: &View, date: f64, clock: &SimulationClock, observer: &Observer, times: &mut StepTimes) -> io::Result<()> {
        let outcome = match state {
            RenderingState::Chars(state) => { render_character_frame(state, &mut self.session, sky, view, date, clock, observer, times)?; crate::model::RenderOutcome::Presented }
            RenderingState::Pixels(state) => render_pixel_frame(state, &mut self.session, sky, None, view, date, clock, observer, times)?,
            RenderingState::Pending => panic!("rendering state is not initialized"),
        };
        times.record_frame_presentation(outcome.was_presented());
        Ok(())
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(Renderer { session });
