//! The frame loop: the seven steps of `dev/pipeline-restructuring-plan.md` section 6, called by name and in
//! order every frame. Keys and resizes are polled first; the simulated time comes from the clock.

use std::io;
use std::thread;
use std::time::Duration;

use astroterm::model::{Config, SkyTables};
use astroterm::observation::{Observation, observe};
use astroterm::points::{PointInputs, draw_points};
use astroterm::present::{Display, present};
use astroterm::projection::project_stars_to_screen;
use astroterm::regions::{RefreshReport, RegionCaches, refresh_regions};
use astroterm::selection::{Selected, select_regions};
use astroterm::solar_system::simulate_solar_system;
use astroterm::terminal::poll_frame_input;
use astroterm::timing::StepTimes;

use crate::helpers::{
    BenchmarkSession, Panel, PanelFrame, apply_frame_controls, begin_benchmark_capture, begin_frame_diagnostics, capture_display,
    complete_benchmark_frame, finish_frame_diagnostics, prepare_benchmark_frame, resolve_frame_time, stop_on_quit, validate_benchmark_time,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_frame_loop(config: &Config, tables: &SkyTables, display: &mut Display, times: &mut StepTimes, benchmark: &mut Option<BenchmarkSession>) -> io::Result<()> {
    let frame_duration = Duration::from_secs_f64(1.0 / f64::from(config.fps));                                  // the target time between frames
    let (observer, atmosphere, settings) = (config.simulation.observer, config.simulation.refraction, &config.render);
    let mut view = config.view;
    let mut caches = RegionCaches::default();                                                                    // the step 4 caches: simulations, group and region images
    let mut frame = display.new_frame();                                                                         // the star layer, the overlay and the ray table
    let mut panel = Panel::new(config, tables);
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
        let solar_system = times.measure("Solar system", || simulate_solar_system(&tables.bodies, time));                      // 1  Sun, Moon, planets
        let observation = times.measure("Observation", || observe(&observer, atmosphere, &view, time));                        // 2  rotation, observed patch, aberration, refraction
        let selected = times.measure("Selection", || select_regions(&tables.stars, &observation));                             // 3  overlapping regions; the constellation table is always included
        let refreshed = times.measure("Regions", || refresh_regions(&mut caches, tables, &selected, time, settings));          // 4  per group: simulate if hold expired, draw if key changed, re-sum region
        times.measure("Projection", || project_stars_to_screen(&mut frame, &caches, &selected, &observation, &view));          // 5  warp region images; per screen pixel, no per-star work
        describe_steps(times, &observation, &selected, refreshed, &caches);
        let lines = panel.lines(&PanelFrame { time, clock: &clock, solar_system: &solar_system, selected: &selected, caches: &caches, refreshed, display: display.info() }, &observer, &view, tables, times);
        let inputs = PointInputs { solar_system: &solar_system, tables, caches: &caches, selected: &selected, panel: lines };
        times.measure("Points", || draw_points(&mut frame, &inputs, &observation, &view, settings));                         // 6  planets, Moon, Sun, constellation lines, labels, overlays
        let presented = times.measure("Present", || present(&frame, display.terminal()))?;                                    // 7  tone map, encode, send; nothing for an unchanged frame

        // after the frame: timings, benchmark, pacing
        let elapsed = finish_frame_diagnostics(config, frame_start, time, presented, times);
        if complete_benchmark_frame(benchmark, time, presented, elapsed, || capture_display(display.info()), times)? || config.debug_singleframe { return Ok(()); }
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                   // wait until the next frame is due, unless already running late
    }
}

/// Attach the selection and refresh results to the single-frame trace; nothing happens without tracing.
fn describe_steps(times: &mut StepTimes, observation: &Observation, selected: &Selected, refreshed: RefreshReport, caches: &RegionCaches) {
    times.describe("Selection", || format!("regions={}; whole sky={}; patch radius={:.2}°", selected.regions.len(), selected.whole_sky, observation.patch.radius.to_degrees()));
    times.describe("Regions", || format!("groups simulated={}; images drawn={}; regions summed={}; tile bytes={}", refreshed.simulated, refreshed.rendered, refreshed.summed, caches.tile_bytes()));
}
