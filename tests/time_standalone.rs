//! Compile time/ as a standalone module: none of the application's other modules exist in this crate.
#[allow(dead_code, unused_imports)]
#[path = "../src/time/mod.rs"]
mod time;

#[test]
fn startup_and_frame_timing_work_without_application_types() {
    let startup = time::StartupTiming::new(true, true);
    let mut manager = startup.start_simulation(2451545.0, 1.0, 60, Some(0.0));
    manager.step_times.begin_frame();
    manager.measure("Frame.Work");
    manager.measure("END");

    assert_eq!(manager.clock.julian_date(), 2451545.0);
    assert_eq!(manager.step_times.steps()[1].name, "Work");
    assert!(manager.step_times.capture_steps().is_ok());
}
