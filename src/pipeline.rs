//! The frame loop: the seven steps of `dev/pipeline-restructuring-plan.md` section 6, called by name and in
//! order every frame. Keys and resizes are polled first; the simulated time comes from the clock.

use std::io;
use std::thread;
use std::time::Duration;

use astroterm::model::{Config, Data};
use astroterm::observation::{Observation, observe};
use astroterm::points::{PointInputs, draw_points};
use astroterm::present::{Display, present, transport_notice};
use astroterm::projection::project_stars_to_screen;
use astroterm::regions::{RefreshReport, refresh_regions};
use astroterm::selection::{Selected, select_regions};
use astroterm::solar_system::simulate_solar_system;
use astroterm::terminal::poll_frame_input;
use astroterm::timing::StepTimes;

use crate::helpers::{
    BenchmarkSession, Panel, PanelFrame, apply_frame_controls, begin_benchmark_capture, begin_frame_diagnostics, capture_display,
    complete_benchmark_frame, finish_frame_diagnostics, prepare_benchmark_frame, resolve_frame_time, stop_on_quit, validate_benchmark_time,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_frame_loop(config: &Config, data: &mut Data, display: &mut Display, times: &mut StepTimes, benchmark: &mut Option<BenchmarkSession>) -> io::Result<()> {
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));                                  // the target time between frames
    let (observer, atmosphere, settings) = (config.simulation.observer, config.simulation.refraction, &config.render);
    let mut view = config.view;
    data.star_opacities.prepare(1.0);                                                                     // prepare the initial shared drawing curve before frame timings
    let mut frame = display.new_frame();                                                                         // the star layer, the overlay and the ray table
    let mut panel = Panel::new(config, data);
    let mut clock = begin_benchmark_capture(benchmark, config, || capture_display(display.info()), times)?;     // export startup timings if requested, then start simulated time
    times.reset_frame_timings();                                                                                 // keep setup time out of the displayed frame timings

    loop {
        let frame_start = begin_frame_diagnostics(times);

        // input: keys, resize; may change the view or the clock
        let input = poll_frame_input(config.terminal.quit_on_any_key)?;
        prepare_benchmark_frame(benchmark, &input, &mut clock)?;                                                 // a benchmark rejects input and sets the frame's date
        if stop_on_quit(&input) { return Ok(()); }
        apply_frame_controls(&input, &config.view, &mut view, &mut clock);
        if input.resized { display.fit(&mut frame)?; }
        let time = resolve_frame_time(config.debug_singleframe, config.simulation.start_julian_date, &clock);
        validate_benchmark_time(benchmark, time)?;

        // the seven steps
        let solar_system = times.measure_steps("Solar system", |times| simulate_solar_system(data, time, times));                                         // 1  Sun, Moon, planets
        let observation = times.measure_steps("Observation", |times| observe(&observer, atmosphere, &view, time, times));                                 // 2  rotation, observed patch, aberration, refraction
        let selected = times.measure_steps("Selection", |times| select_regions(&data.star_regions, &observation, times));                                 // 3  overlapping regions; the constellation table is always included
        let refreshed = times.measure_steps("Regions", |times| refresh_regions(data, &selected, time, &view, settings, times));                           // 4  per group: simulate if hold expired, draw if key changed, re-merge region
        times.measure_steps("Projection", |times| project_stars_to_screen(&mut frame, data, &selected, &observation, &view, times));                      // 5  project cached region images, no per-star work
        describe_steps(times, &observation, &selected, refreshed, data);
        let info = display.info();                                                                                                 // the transport as it is now: it can fall back mid-run
        let lines = panel.lines(&PanelFrame { time, clock: &clock, solar_system: &solar_system, selected: &selected, refreshed, display: info }, &observer, &view, data, times);
        let inputs = PointInputs { solar_system: &solar_system, data, selected: &selected, panel: lines, notice: transport_notice(&info) };
        times.measure_steps("Points", |times| draw_points(&mut frame, &inputs, &observation, &view, settings, times));                                    // 6  planets, Moon, Sun, constellation lines, labels, overlays
        let presented = times.measure_steps("Present", |times| present(&frame, display.terminal(), times))?;                                              // 7  tone map, encode, send; nothing for an unchanged frame

        // after the frame: timings, benchmark, pacing
        let elapsed = finish_frame_diagnostics(config, frame_start, time, presented, times);
        if complete_benchmark_frame(benchmark, time, presented, elapsed, || capture_display(display.info()), times)? || config.debug_singleframe { return Ok(()); }
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                   // wait until the next frame is due, unless already running late
    }
}

/// Attach the selection and refresh results to the single-frame trace; nothing happens without tracing.
fn describe_steps(times: &mut StepTimes, observation: &Observation, selected: &Selected, refreshed: RefreshReport, data: &Data) {
    times.describe("Selection", || format!("regions={}; whole sky={}; patch radius={:.2}°", selected.regions.len(), selected.whole_sky, observation.patch.radius.to_degrees()));
    times.describe("Regions", || format!("groups simulated={}; images drawn={}; regions merged={}; tile bytes={}", refreshed.simulated, refreshed.rendered, refreshed.merged, data.tile_bytes));
}
