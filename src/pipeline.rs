//! The frame loop: the seven steps of `dev/pipeline-restructuring-plan.md` section 6, called by name and in
//! order every frame. Keys and resizes are polled first; the simulated time comes from the clock.

use std::io;
use std::thread;

use astroterm::model::{Config, Data};
use astroterm::observation::{Observation, observe};
use astroterm::points::{PointInputs, draw_points};
use astroterm::present::{Display, present, transport_notice};
use astroterm::projection::{Frame, project_stars_to_screen};
use astroterm::regions::{RefreshReport, refresh_regions};
use astroterm::selection::{Selected, select_regions};
use astroterm::solar_system::simulate_solar_system;
use astroterm::terminal::poll_frame_input;
use astroterm::timing::StepTimes;

use crate::time::TimeManager;

use crate::helpers::{
    BenchmarkSession, Panel, PanelFrame, apply_frame_controls, begin_frame_diagnostics, capture_display,
    complete_benchmark_frame, finish_frame_diagnostics, prepare_benchmark_frame, resolve_frame_time, stop_on_quit, validate_benchmark_time,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_frame_loop(config: &Config, data: &mut Data, display: &mut Display, frame: &mut Frame, panel: &mut Panel, time_manager: &mut TimeManager, benchmark: &mut Option<BenchmarkSession>) -> io::Result<()> {
    let (observer, atmosphere, settings) = (config.simulation.observer, config.simulation.refraction, &config.render);
    let mut view = config.view;

    time_manager.step_times.reset_frame_timings(); // keep setup time out of the displayed frame timings

    loop {
        let frame_start = begin_frame_diagnostics(&mut time_manager.step_times);

        // input: keys, resize; may change the view or the clock
        let input = poll_frame_input(config.terminal.quit_on_any_key)?;
        prepare_benchmark_frame(benchmark, &input, &mut time_manager.clock)?;                                                 // a benchmark rejects input and sets the frame's date
        if stop_on_quit(&input) { return Ok(()); }
        apply_frame_controls(&input, &config.view, &mut view, &mut time_manager.clock);
        if input.resized { display.fit(frame)?; }
        let time = resolve_frame_time(config.debug_singleframe, config.simulation.start_julian_date, &time_manager.clock);
        validate_benchmark_time(benchmark, time)?;

        // Run the seven steps directly; each checkpoint finishes the preceding interval.
        time_manager.measure("Solar system");
        let solar_system = simulate_solar_system(data, time, &mut time_manager.step_times);
        time_manager.measure("Observation");
        let observation = observe(&observer, atmosphere, &view, time, &mut time_manager.step_times);
        time_manager.measure("Selection");
        let selected = select_regions(&data.star_regions, &observation, &mut time_manager.step_times);
        time_manager.measure("Regions");
        let refreshed = refresh_regions(data, &selected, time, &view, settings, &mut time_manager.step_times);
        time_manager.measure("Projection");
        project_stars_to_screen(frame, data, &selected, &observation, &view, &mut time_manager.step_times);

        // Include the panel and trace descriptions in the work needed to prepare point overlays.
        time_manager.measure("Points");
        describe_steps(&mut time_manager.step_times, &observation, &selected, refreshed, data);
        let info = display.info();
        let lines = panel.lines(&PanelFrame { time, clock: &time_manager.clock, solar_system: &solar_system, selected: &selected, refreshed, display: info }, &observer, &view, data, &time_manager.step_times);
        let inputs = PointInputs { solar_system: &solar_system, data, selected: &selected, panel: lines, notice: transport_notice(&info) };
        draw_points(frame, &inputs, &observation, &view, settings, &mut time_manager.step_times);

        // Stop timing on both successful and failed writes, before reporting or sleeping.
        time_manager.measure("Present");
        let result = present(frame, display.terminal(), &mut time_manager.step_times);
        time_manager.measure("END");
        let presented = result?;

        // after the frame: timings, benchmark, pacing
        let elapsed = finish_frame_diagnostics(config, frame_start, time, presented, &mut time_manager.step_times);
        if complete_benchmark_frame(benchmark, time, presented, elapsed, || capture_display(display.info()), &mut time_manager.step_times)? || config.debug_singleframe { return Ok(()); }
        let frame_duration = &time_manager.min_frame_duration;
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                   // wait until the next frame is due, unless already running late
    }
}

/// Attach the selection and refresh results to the single-frame trace; nothing happens without tracing.
fn describe_steps(times: &mut StepTimes, observation: &Observation, selected: &Selected, refreshed: RefreshReport, data: &Data) {
    times.describe("Selection", || format!("regions={}; whole sky={}; patch radius={:.2}°", selected.regions.len(), selected.whole_sky, observation.patch.radius.to_degrees()));
    times.describe("Regions", || format!("groups simulated={}; images drawn={}; regions merged={}; tile bytes={}", refreshed.simulated, refreshed.rendered, refreshed.merged, data.tile_bytes));
}
