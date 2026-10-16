//! The frame loop: the seven steps of `dev/pipeline-restructuring-plan.md` section 6, called by name and in
//! order every frame, with a separate metadata text-overlay pass between points and presentation.
//! Keys and resizes are polled first; the simulated time comes from the clock.

use std::io;
use std::thread;

use astroterm::model::{Config, Data};
use astroterm::observation::observe;
use astroterm::points::{draw_points};
use astroterm::present::{Display, present, transport_notice};
use astroterm::projection::{Frame, ProjectionExecutor, project_stars_to_screen};
use astroterm::regions::{RefreshReport, refresh_regions};
use astroterm::selection::{Selected, select_regions};
use astroterm::solar_system::simulate_solar_system;
use astroterm::time::{StepTimes, TimeManager};

use crate::helpers::{
    BenchmarkSession, Panel, draw_text_overlay, handle_input, begin_frame_diagnostics,
    complete_benchmark_frame, finish_frame_diagnostics,
};

/// Draw frames at the configured rate until the user quits. Keys change the view and the simulation clock.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_frame_loop(
    config: &Config,
    data: &mut Data,
    display: &mut Display,
    frame: &mut Frame,
    panel: &mut Panel,
    time_manager: &mut TimeManager,
    projection: &mut ProjectionExecutor,
    benchmark: &mut Option<BenchmarkSession>
)-> io::Result<()> {

    let mut current_view = config.view;
    time_manager.step_times.reset_frame_timings();

    loop {
        let frame_start = begin_frame_diagnostics(&mut time_manager.step_times);

        let should_quit = handle_input(config, &mut current_view, &mut time_manager.clock, display, frame, benchmark)?;
        if should_quit { return Ok(()); }

        let time_ms = time_manager.get_current_simulation_time()?;

        time_manager.measure("Solar system");
        let solar_system = simulate_solar_system(&data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons, time_ms, &mut time_manager.step_times);

        time_manager.measure("Observation");
        let observation = observe(&config.simulation.observer, config.simulation.refraction, &current_view, time_ms, solar_system.earth, &mut time_manager.step_times);

        time_manager.measure("Selection");
        let selected = select_regions(&data.star_regions, &observation.patch, observation.time_ms, &mut time_manager.step_times);

        time_manager.measure("Regions");
        let refreshed = refresh_regions(
            &mut data.star_regions, &data.constellation_stars,
            &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
            &mut data.star_opacities, &mut data.tile_bytes,
            &selected.regions, time_ms, current_view.fov_degrees, config.render.magnitude_threshold,
            &mut time_manager.step_times,
        );

        time_manager.measure("Projection");
        let projected_stars = project_stars_to_screen(frame.drawing_area_width_and_height, &data.star_regions, &selected.regions, &observation, &current_view, projection, &mut time_manager.step_times);
        time_manager.measure("Projection.Frame insertion");
        frame.insert_projected_stars(projected_stars);

        time_manager.measure("Points");
        let drawn = draw_points(
            frame, &solar_system,
            &data.solar_system_sun, &data.solar_system_planets, &data.solar_system_moons,
            &data.constellation_stars, &data.constellation_simulation, &data.constellation_figures,
            &data.star_regions, &data.names, &data.star_opacities, &data.clipping,
            &selected.regions, transport_notice(&display.info()),
            &observation, &current_view, &config.render, &mut time_manager.step_times,
        );

        time_manager.measure("Text overlay");
        draw_text_overlay(
            frame, display, panel, &solar_system, &selected,
            &data.star_regions, data.tile_bytes, &drawn.notices, refreshed,
            time_ms, &current_view, config, &time_manager.clock, &mut time_manager.step_times,
        );

        time_manager.measure("Present");
        let presentation_result = present(frame, display.terminal(), &mut time_manager.step_times);

        time_manager.measure("END");
        let presented = presentation_result?;

        // Record the completed frame's timings and any requested benchmark sample.
        let elapsed = finish_frame_diagnostics(config, frame_start, time_ms, presented, &mut time_manager.step_times);
        describe_steps(&mut time_manager.step_times, observation.patch.radius, &selected, refreshed, data.tile_bytes);
        let benchmark_complete = complete_benchmark_frame(benchmark, time_ms, presented, elapsed, display, &mut time_manager.step_times)?;

        // Stop after the requested benchmark frames or the single diagnostic frame.
        if benchmark_complete || config.debug_singleframe { return Ok(()); }

        // Wait for the next frame's scheduled start; a late frame needs no extra delay.
        let frame_duration = &time_manager.min_frame_duration;
        thread::sleep(frame_duration.saturating_sub(frame_start.elapsed()));
    }
}

/// Attach the selection and refresh results to the single-frame trace; nothing happens without tracing.
fn describe_steps(times: &mut StepTimes, patch_radius: f64, selected: &Selected, refreshed: RefreshReport, tile_bytes: usize) {
    times.describe("Selection", || format!("regions={}; whole sky={}; patch radius={:.2}°", selected.regions.len(), selected.whole_sky, patch_radius.to_degrees()));
    times.describe("Regions", || format!("groups simulated={}; images drawn={}; regions merged={}; tile bytes={}", refreshed.simulated, refreshed.rendered, refreshed.merged, tile_bytes));
}
