//! Concrete renderer selection; shared input and sky stages remain in main.rs.
use super::{TerminalRenderer, TerminalSettings, open_terminal_renderer, pixels::PixelRenderer};
use crate::{
    astro::{Observer, SimulationClock},
    projection::{ProjectedSky, View, Viewport},
    scene::RenderOptions,
    timing::StepTimes,
};
use ratatui_image::picker::ProtocolType;
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum RendererKind {
    #[default]
    Chars,
    Pixels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphicsProtocol {
    #[default]
    Auto,
    Kitty,
    Sixel,
    Iterm2,
    Halfblocks,
}
impl GraphicsProtocol {
    fn forced(self) -> Option<ProtocolType> {
        match self {
            Self::Auto => None,
            Self::Kitty => Some(ProtocolType::Kitty),
            Self::Sixel => Some(ProtocolType::Sixel),
            Self::Iterm2 => Some(ProtocolType::Iterm2),
            Self::Halfblocks => Some(ProtocolType::Halfblocks),
        }
    }
}

pub enum Renderer {
    Chars(Box<TerminalRenderer>),
    Pixels(Box<PixelRenderer>),
}
impl Renderer {
    pub fn open(
        kind: RendererKind,
        protocol: GraphicsProtocol,
        options: RenderOptions,
        settings: TerminalSettings,
        text_scale: f64,
    ) -> io::Result<Self> {
        if kind == RendererKind::Pixels {
            match PixelRenderer::open(options, settings, protocol.forced(), text_scale) {
                Ok(renderer) => return Ok(Self::Pixels(Box::new(renderer))),
                Err(error) => {
                    let mut renderer = open_terminal_renderer(options, settings)?;
                    renderer.startup_notice = Some(format!("Pixel startup failed; using characters: {error}"));
                    return Ok(Self::Chars(Box::new(renderer)));
                }
            }
        }
        open_terminal_renderer(options, settings).map(|renderer| Self::Chars(Box::new(renderer)))
    }
    pub fn viewport(&self) -> Viewport {
        match self {
            Self::Chars(renderer) => renderer.viewport(),
            Self::Pixels(renderer) => renderer.viewport(),
        }
    }
    pub fn fit_to_terminal(&mut self) -> io::Result<()> {
        match self {
            Self::Chars(renderer) => renderer.fit_to_terminal(),
            Self::Pixels(renderer) => renderer.fit_to_terminal(),
        }
    }
    pub fn render_frame(
        &mut self,
        sky: &ProjectedSky<'_>,
        view: &View,
        date: f64,
        clock: &SimulationClock,
        observer: &Observer,
        times: &mut StepTimes,
    ) -> io::Result<()> {
        match self {
            Self::Chars(renderer) => renderer.render_frame(sky, view, date, clock, observer, times),
            Self::Pixels(renderer) => renderer.render_frame(sky, view, date, clock, observer, times),
        }
    }
}
