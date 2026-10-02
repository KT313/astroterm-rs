//! The terminal renderer: draws the sky as a grid of characters and shows it in the terminal.

use std::io;

use crate::astro::{Observer, SimulationClock};
use crate::metadata::collect_metadata_fields;
use crate::projection::View;
use crate::scene::{RenderOptions, draw_metadata_panel, draw_sky_scene};
use crate::sky::Sky;

use super::present::Frame;
use super::session::{TerminalSession, open_terminal_session};

/// Settings of the terminal itself, as opposed to what is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalSettings {
    /// Cell height / width; detected from the terminal when `None`.
    pub aspect_ratio: Option<f64>,
    /// Show the metadata panel in the top left corner.
    pub metadata_panel: bool,
    /// Quit on any key instead of only `q`, Esc and Ctrl-C.
    pub quit_on_any_key: bool,
}

/// Renders frames into the terminal for as long as it exists. The terminal is restored when it is dropped.
pub struct TerminalRenderer {
    session: TerminalSession,
    frame: Frame,
    options: RenderOptions,
    settings: TerminalSettings,
}

/// Take over the terminal and size the canvases to it.
pub fn open_terminal_renderer(options: RenderOptions, settings: TerminalSettings) -> io::Result<TerminalRenderer> {
    let mut session = open_terminal_session()?;
    let frame = session.fit_frame(settings.aspect_ratio, settings.metadata_panel)?;
    Ok(TerminalRenderer {
        session,
        frame,
        options,
        settings,
    })
}

impl TerminalRenderer {
    /// Resize the canvases to the terminal, after it was resized. The next frame is drawn in full.
    pub fn fit_to_terminal(&mut self) -> io::Result<()> {
        self.frame = self
            .session
            .fit_frame(self.settings.aspect_ratio, self.settings.metadata_panel)?;
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
            let fields =
                collect_metadata_fields(julian_date, clock, sky.moon.phase, observer, view, self.options.unicode);
            draw_metadata_panel(panel, &fields);
        }
        self.session.present(&self.frame)
    }
}
