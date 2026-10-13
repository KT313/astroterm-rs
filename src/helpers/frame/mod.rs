//! What the frame loop does around the seven steps: controls and the clock, and the metadata panel's lines.
use astroterm::astro::{FrameTime, Observer, SimulationClock, name_moon_phase};
use astroterm::controls::{Control, apply_control};
use astroterm::metadata::{MetadataField, ObserverTimeZone, append_step_time_fields, fill_metadata_fields, format_star_count, resolve_observer_timezone};
use astroterm::model::{Config, SkyTables, View};
use astroterm::present::{DisplayInfo, describe_compression};
use astroterm::regions::{RefreshReport, RegionCaches};
use astroterm::selection::Selected;
use astroterm::solar_system::SolarSystem;
use astroterm::terminal::FrameInput;
use astroterm::timing::StepTimes;
use ratatui_image::picker::ProtocolType;

/// Apply a frame's controls to the view and the clock. The caller checks for quit first.
pub(crate) fn apply_frame_controls(input: &FrameInput, initial_view: &View, view: &mut View, clock: &mut SimulationClock) {
    for &control in &input.controls {
        apply_control(control, view, clock, initial_view);
    }
}

/// Use the exact requested epoch for single-frame diagnostics, otherwise read the running clock.
pub(crate) fn resolve_frame_time(single_frame: bool, start_julian_date: f64, clock: &SimulationClock) -> FrameTime {
    FrameTime::from_utc(if single_frame { start_julian_date } else { clock.julian_date() })
}

/// Stop on a quit key without processing any other control.
pub(crate) fn stop_on_quit(input: &FrameInput) -> bool {
    input.controls.contains(&Control::Quit)
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
    pub time: FrameTime,
    pub clock: &'a SimulationClock,
    pub solar_system: &'a SolarSystem,
    pub selected: &'a Selected,
    pub caches: &'a RegionCaches,
    pub refreshed: RefreshReport,
    pub display: DisplayInfo,
}

impl Panel {
    pub(crate) fn new(config: &Config, tables: &SkyTables) -> Self {
        let enabled = config.terminal.metadata_panel;
        Self {
            frame_times: config.terminal.frame_times,
            unicode: config.render.unicode,
            time_zone: enabled.then(|| resolve_observer_timezone(&config.simulation.observer)),
            stars_loaded: tables.stars.len() + tables.constellations.len(),
            lines: Vec::new(),
        }
    }

    /// Rebuild the lines for this frame: local time and view (as before), the catalog and region counts, the
    /// display, and the step timings when requested.
    pub(crate) fn lines(&mut self, frame: &PanelFrame<'_>, observer: &Observer, view: &View, tables: &SkyTables, times: &StepTimes) -> &[MetadataField] {
        self.lines.clear();
        let Some(time_zone) = &self.time_zone else { return &self.lines; };
        let phase = name_moon_phase(frame.solar_system.moon_illumination(0));
        fill_metadata_fields(&mut self.lines, frame.time.utc, frame.clock, phase, observer, view, self.unicode, time_zone);

        // stars and regions
        let in_regions: usize = frame.selected.regions.iter().map(|&region| tables.stars.region(region).iter().map(|table| table.len()).sum::<usize>()).sum();
        let field = |label: &str, value: String| MetadataField { label: label.into(), value };
        self.lines.push(field("Stars loaded", format!("{} stars", format_star_count(self.stars_loaded))));
        self.lines.push(field("In selected regions", format!("{} stars", format_star_count(in_regions))));
        self.lines.push(field("Regions selected", format!("{}{}", frame.selected.regions.len(), if frame.selected.whole_sky { " (whole sky)" } else { "" })));
        self.lines.push(field("Tile memory", format!("{:.1} MiB", frame.caches.tile_bytes() as f64 / (1024.0 * 1024.0))));

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
