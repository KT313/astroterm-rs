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
use astroterm::time::{StepTimes, TimeManager};

use crate::helpers::{
    BenchmarkSession, Panel, PanelFrame, handle_input, begin_frame_diagnostics, capture_display,
    complete_benchmark_frame, finish_frame_diagnostics,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
pub(super) fn run_frame_loop(
    config: &Config,
    data: &mut Data,
    display: &mut Display,
    frame: &mut Frame,
    panel: &mut Panel,
    time_manager: &mut TimeManager,
    benchmark: &mut Option<BenchmarkSession>
)-> io::Result<()> {

    let mut current_view = config.view;
    time_manager.step_times.reset_frame_timings();

    loop {
        let frame_start = begin_frame_diagnostics(&mut time_manager.step_times);

        let should_quit = handle_input(config, &mut current_view, &mut time_manager.clock, display, frame, benchmark)?;
        if should_quit { return Ok(()); }

        // Read the single millisecond timestamp used by every stage of this frame.
        let time_ms = time_manager.get_current_simulation_time()?;

        time_manager.measure("Solar system");
        let solar_system = simulate_solar_system(data, time_ms, &mut time_manager.step_times);

        time_manager.measure("Observation");
        let observation = observe(&config.simulation.observer, config.simulation.refraction, &current_view, time_ms, &mut time_manager.step_times);

        time_manager.measure("Selection");
        let selected = select_regions(&data.star_regions, &observation, &mut time_manager.step_times);

        time_manager.measure("Regions");
        let refreshed = refresh_regions(data, &selected, time_ms, &current_view, &config.render, &mut time_manager.step_times);

        time_manager.measure("Projection");
        project_stars_to_screen(frame, data, &selected, &observation, &current_view, &mut time_manager.step_times);


        // Include the panel and trace descriptions in the work needed to prepare point overlays.
        time_manager.measure("Points");
        describe_steps(&mut time_manager.step_times, &observation, &selected, refreshed, data);
        let info = display.info();
        let lines = panel.lines(&PanelFrame { time_ms, clock: &time_manager.clock, solar_system: &solar_system, selected: &selected, refreshed, display: info }, &config.simulation.observer, &current_view, data, &time_manager.step_times);
        let inputs = PointInputs { solar_system: &solar_system, data, selected: &selected, panel: lines, notice: transport_notice(&info) };
        draw_points(frame, &inputs, &observation, &current_view, &config.render, &mut time_manager.step_times);

        // Stop timing on both successful and failed writes, before reporting or sleeping.
        time_manager.measure("Present");
        let result = present(frame, display.terminal(), &mut time_manager.step_times);
        time_manager.measure("END");
        let presented = result?;

        // after the frame: timings, benchmark, pacing
        let elapsed = finish_frame_diagnostics(config, frame_start, time_ms, presented, &mut time_manager.step_times);
        if complete_benchmark_frame(benchmark, time_ms, presented, elapsed, || capture_display(display.info()), &mut time_manager.step_times)? || config.debug_singleframe { return Ok(()); }
        let frame_duration = &time_manager.min_frame_duration;
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));                                   // wait until the next frame is due, unless already running late
    }
}

/// Attach the selection and refresh results to the single-frame trace; nothing happens without tracing.
fn describe_steps(times: &mut StepTimes, observation: &Observation, selected: &Selected, refreshed: RefreshReport, data: &Data) {
    times.describe("Selection", || format!("regions={}; whole sky={}; patch radius={:.2}°", selected.regions.len(), selected.whole_sky, observation.patch.radius.to_degrees()));
    times.describe("Regions", || format!("groups simulated={}; images drawn={}; regions merged={}; tile bytes={}", refreshed.simulated, refreshed.rendered, refreshed.merged, data.tile_bytes));
}
