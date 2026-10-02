//! The terminal renderer: draws the sky as a grid of characters and shows it in the terminal.

use std::io;

use crate::astro::{Observer, SimulationClock};
use crate::projection::View;
use crate::scene::{RenderOptions, draw_metadata, draw_sky_scene};
use crate::sky::Sky;

use super::present::Frame;
use super::session::{TerminalSession, open_terminal_session};

/// Renders frames into the terminal for as long as it exists. The terminal is restored when it is dropped.
pub struct TerminalRenderer {
    session: TerminalSession,
    frame: Frame,
    options: RenderOptions,
    /// Cell height / width; detected from the terminal when `None`.
    aspect_ratio: Option<f64>,
    with_panel: bool,
}

/// Take over the terminal and size the canvases to it. `with_panel` adds the metadata panel.
pub fn open_terminal_renderer(
    options: RenderOptions,
    aspect_ratio: Option<f64>,
    with_panel: bool,
) -> io::Result<TerminalRenderer> {
    let mut session = open_terminal_session()?;
    let frame = session.fit_frame(aspect_ratio, with_panel)?;
    Ok(TerminalRenderer {
        session,
        frame,
        options,
        aspect_ratio,
        with_panel,
    })
}

impl TerminalRenderer {
    /// Resize the canvases to the terminal, after it was resized. The next frame is drawn in full.
    pub fn fit_to_terminal(&mut self) -> io::Result<()> {
        self.frame = self.session.fit_frame(self.aspect_ratio, self.with_panel)?;
        Ok(())
    }

    /// Draw the sky as seen in `view`, with the metadata panel on top, and show it. `julian_date` is the simulation
    /// time the sky's positions were computed for.
    pub fn render_frame(
        &mut self,
        sky: &Sky,
        view: &View,
        julian_date: f64,
        clock: &SimulationClock,
        observer: &Observer,
    ) -> io::Result<()> {
        draw_sky_scene(&mut self.frame.sky, view, &self.options, sky);
        if let Some(panel) = &mut self.frame.panel {
            draw_metadata(
                panel,
                julian_date,
                clock,
                sky.moon.phase,
                observer,
                view,
                self.options.unicode,
            );
        }
        self.session.present(&self.frame)
    }
}
