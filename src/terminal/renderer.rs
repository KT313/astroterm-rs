//! The terminal renderer: draws the sky as a grid of characters and shows it in the terminal.

use std::io;

use crate::astro::{Observer, SimulationClock};
use crate::metadata::{ObserverTimeZone, collect_metadata_fields, format_step_time_fields};
use crate::projection::View;
use crate::scene::{RenderOptions, draw_metadata_panel, draw_sky_scene};
use crate::sky::Sky;
use crate::timing::StepTimes;

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
    /// Show the frame step durations below the metadata.
    pub frame_times: bool,
}

/// Renders frames into the terminal for as long as it exists. The terminal is restored when it is dropped.
pub struct TerminalRenderer {
    session: TerminalSession,
    frame: Frame,
    options: RenderOptions,
    settings: TerminalSettings,
    time_zone: Option<(Observer, ObserverTimeZone)>,
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
        time_zone: None,
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

    /// Draw the sky as seen in `view`, with the metadata panel on top, and show it. `julian_date_utc` is the simulation
    /// time the sky's positions were computed for. The durations of drawing and presenting are added to `step_times`;
    /// the panel shows them as of the previous frame.
    pub fn render_frame(
        &mut self,
        sky: &Sky,
        view: &View,
        julian_date_utc: f64,
        clock: &SimulationClock,
        observer: &Observer,
        step_times: &mut StepTimes,
    ) -> io::Result<()> {
        // resolve geographic zone rules only when a panel needs them and the observer changes
        if self.frame.panel.is_some() && self.time_zone.as_ref().is_none_or(|(site, _)| site != observer) {
            self.time_zone = Some((*observer, ObserverTimeZone::new(observer)));
        }

        // the step durations so far, read before this frame's drawing is measured
        let step_time_fields = self
            .settings
            .frame_times
            .then(|| format_step_time_fields(step_times.steps()));

        // draw the sky and the panel, then write the changes to the terminal
        step_times.measure("Draw", || {
            draw_sky_scene(&mut self.frame.sky, view, &self.options, sky);
            if let Some(panel) = &mut self.frame.panel {
                let mut fields = collect_metadata_fields(
                    julian_date_utc,
                    clock,
                    sky.moon.phase,
                    observer,
                    view,
                    self.options.unicode,
                    &self.time_zone.as_ref().expect("panel zone initialized").1,
                );
                fields.extend(step_time_fields.into_iter().flatten());
                draw_metadata_panel(panel, &fields);
            }
        });
        step_times.measure("Present", || self.session.present(&self.frame))
    }
}
