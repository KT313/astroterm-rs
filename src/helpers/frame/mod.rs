//! Frame-loop helpers: controls and the clock, plus preparation and drawing of the metadata text overlay.
use astroterm::time::{SimulationClock, TimeManager};
use astroterm::points::draw_panel_and_notices;
use astroterm::astro::{Observer, name_moon_phase};
use astroterm::controls::{Control, apply_control};
use astroterm::metadata::{MetadataField, ObserverTimeZone, append_step_time_fields, fill_metadata_fields, format_star_count, resolve_observer_timezone};
use astroterm::model::{Config, Data, View};
use astroterm::present::{Display, DisplayInfo, describe_compression};
use astroterm::projection::Frame;
use astroterm::regions::RefreshReport;
use astroterm::selection::Selected;
use astroterm::solar_system::SolarSystem;
use astroterm::terminal::{FrameInput, poll_frame_input};
use astroterm::time::StepTimes;
use ratatui_image::picker::ProtocolType;
use std::io;

use super::{BenchmarkSession, set_benchmark_frame_time, validate_benchmark_input};

/// Handle this frame's input and benchmark clock. Return true when the frame loop should quit.
pub(crate) fn handle_input(
    config: &Config,
    current_view: &mut View,
    clock: &mut SimulationClock,
    display: &mut Display,
    frame: &mut Frame,
    benchmark: &Option<BenchmarkSession>,
) -> io::Result<bool> {
    // Read pending key presses and terminal resize events.
    let input = poll_frame_input(config.terminal.quit_on_any_key)?;

    // Benchmarks reject input changes before setting the scheduled date for this frame.
    validate_benchmark_input(benchmark, &input)?;
    set_benchmark_frame_time(benchmark, clock)?;

    // Stop on quit before applying other controls to the current view or simulation clock.
    if stop_on_quit(&input) {
        return Ok(true);
    }
    apply_frame_controls(&input, &config.view, current_view, clock)?;

    // Resize the drawing buffers when the terminal dimensions have changed.
    if input.resized {
        display.fit(frame)?;
    }
    Ok(false)
}

/// Apply a frame's controls to the view and the clock. The caller checks for quit first.
pub(crate) fn apply_frame_controls(input: &FrameInput, initial_view: &View, view: &mut View, clock: &mut SimulationClock) -> io::Result<()> {
    for &control in &input.controls {
        apply_control(control, view, clock, initial_view)?;
    }
    Ok(())
}

/// Stop on a quit key without processing any other control.
pub(crate) fn stop_on_quit(input: &FrameInput) -> bool {
    input.controls.contains(&Control::Quit)
}

/// Build and paint the metadata panel after sky graphics, keeping notices on top.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_text_overlay(
    frame: &mut Frame,
    display: &Display,
    panel: &mut Panel,
    solar_system: &SolarSystem,
    selected: &Selected,
    data: &Data,
    notices: &[&str],
    refreshed: RefreshReport,
    time_ms: i64,
    view: &View,
    config: &Config,
    time_manager: &mut TimeManager,
) {
    // Build the panel's text from the simulation results and current terminal information.
    time_manager.measure("Text overlay.Panel preparation");
    let display_info = display.info();
    let panel_lines = panel.lines(
        &PanelFrame {
            time_ms,
            clock: &time_manager.clock,
            solar_system,
            selected,
            refreshed,
            display: display_info,
        },
        &config.simulation.observer,
        view,
        data,
        &time_manager.step_times,
    );

    // Paint the panel and notices without clearing the graphics already in the frame.
    time_manager.measure("Text overlay.Text rasterization");
    draw_panel_and_notices(frame, panel_lines, notices);
}

/// The metadata panel: its lines are rebuilt every frame into the same vector, empty when the panel is off.
pub(crate) struct Panel {
    frame_times: bool,
    unicode: bool,
    time_zone: Option<ObserverTimeZone>,          // resolved once, only when the panel is shown
    stars_loaded: usize,
    lines: Vec<MetadataField>,
}

/// What one frame contributes to the panel besides the time and the view.
pub(crate) struct PanelFrame<'a> {
    pub time_ms: i64,
    pub clock: &'a SimulationClock,
    pub solar_system: &'a SolarSystem,
    pub selected: &'a Selected,
    pub refreshed: RefreshReport,
    pub display: DisplayInfo,
}

impl Panel {
    pub(crate) fn new(config: &Config, data: &Data) -> Self {
        let enabled = config.terminal.metadata_panel;
        Self {
            frame_times: config.terminal.frame_times,
            unicode: config.render.unicode,
            time_zone: enabled.then(|| resolve_observer_timezone(&config.simulation.observer)),
            stars_loaded: data.region_star_count() + data.constellation_stars.len(),
            lines: Vec::new(),
        }
    }

    /// Rebuild the lines for this frame: local time and view (as before), the catalog and region counts, the
    /// display, and the step timings when requested.
    pub(crate) fn lines(&mut self, frame: &PanelFrame<'_>, observer: &Observer, view: &View, data: &Data, times: &StepTimes) -> &[MetadataField] {
        self.lines.clear();
        let Some(time_zone) = &self.time_zone else { return &self.lines; };
        let phase = name_moon_phase(frame.solar_system.moon_illumination(0));
        fill_metadata_fields(&mut self.lines, frame.time_ms, frame.clock, phase, observer, view, self.unicode, time_zone);

        // stars and regions
        let in_regions: usize = frame.selected.regions.iter().map(|&region| data.star_regions[region].speed_groups.iter().map(|group| &group.stars).map(|table| table.len()).sum::<usize>()).sum();
        let field = |label: &str, value: String| MetadataField { label: label.into(), value };
        self.lines.push(field("Stars loaded", format!("{} stars", format_star_count(self.stars_loaded))));
        self.lines.push(field("In selected regions", format!("{} stars", format_star_count(in_regions))));
        self.lines.push(field("Regions selected", format!("{}{}", frame.selected.regions.len(), if frame.selected.whole_sky { " (whole sky)" } else { "" })));
        self.lines.push(field("Tile memory", format!("{:.1} MiB", data.tile_bytes as f64 / (1024.0 * 1024.0))));

        // the display
        let display = frame.display;
        self.lines.push(field("Graphics", format!("{:?} · {}×{}", display.protocol, display.image.0, display.image.1)));
        if display.protocol == ProtocolType::Kitty {
            self.lines.push(field("Transport", if display.shared_memory { "Shared memory" } else { "Streaming" }.into()));
            self.lines.push(field("Compression", if display.shared_memory { "Not used (shared memory)".into() } else { describe_compression(display.compression).into() }));
        }

        // timings
        if self.frame_times {
            self.lines.push(field("Groups refreshed", format!("{} simulated, {} drawn, {} merged", frame.refreshed.simulated, frame.refreshed.rendered, frame.refreshed.merged)));
            append_step_time_fields(&mut self.lines, times.steps());
        }
        &self.lines
    }
}

#[cfg(test)]
mod tests;
