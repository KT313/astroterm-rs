use super::*;
use astroterm::{cli::{Arguments, build_config}, model::StarCounts, state::ApplicationState};
use clap::Parser;

#[test]
fn each_stage_publishes_only_its_counter_on_refresh_and_cache_hits() {
    let config = build_config(Arguments::try_parse_from(["astroterm", "-d", "2025-03-01T11:00:00", "-s", "0"]).unwrap(), &[]).unwrap();
    let mut state = ApplicationState::new(config, StepTimes::default());
    state.replace_catalog(astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog().unwrap()).unwrap());
    let loaded = state.persistent.catalog.stars.len();
    assert_eq!(state.star_counts, StarCounts { loaded, ..Default::default() });
    let time = FrameTime::from_utc(state.config.simulation.start_julian_date);
    astroterm::sky::prepare_stellar_catalog(&mut state.cache.simulation.stars, state.persistent.catalog.clone(), time.tt, &mut state.timings);
    let mut previous = None;
    for frame in 0..4 {
        if frame == 2 { state.current_view.fov_degrees = 40.0; }
        let threshold = if frame == 3 { -20.0 } else { 5.0 };
        state.star_counts.candidate_regions = usize::MAX;
        state.star_counts.filtered = usize::MAX;
        state.star_counts.projected = usize::MAX;
        simulate_solar_system_frame(&mut state.cache.simulation.solar_system, &mut state.cache.observer, state.config.simulation.observer, time, &mut state.timings).unwrap();
        let observer = prepare_observer_frame(state.config.simulation.observer, &mut state.cache.simulation.solar_system, &mut state.cache.observer, time, &mut state.timings).unwrap();
        select_stars_frame(&mut state.cache.selection, &state.persistent.catalog, &observer, &state.current_view, threshold, false, &mut state.star_counts.candidate_regions, &mut state.timings);
        assert!(state.star_counts.candidate_regions <= loaded);
        assert_eq!(state.star_counts.filtered, usize::MAX);
        simulate_stars_frame(&mut state.cache.simulation.stars, state.cache.selection.stars(), time, &mut state.timings);
        let observed = observe_frame(&mut state.cache.observation, state.cache.simulation.stars.results(state.cache.selection.stars()), state.cache.observer.bodies(&observer), &observer, threshold, false, &mut state.cache.sky, &mut state.star_counts.filtered, &mut state.timings);
        assert_eq!(state.star_counts.filtered, observed.sky().stars.iter().filter(|s| s.drawable).count());
        assert_eq!(state.star_counts.projected, usize::MAX);
        let viewport = ProjectionViewport { width: 80, height: 40 };
        project_frame(observed, &state.current_view, viewport, time, &mut state.cache.projection, &mut state.star_counts.projected, &mut state.timings);
        let projected = astroterm::projection::borrow_projected(&state.cache.projection, observed.sky(), &state.current_view, viewport);
        assert_eq!(state.star_counts.projected, projected.stars.len());
        assert_eq!(state.star_counts.loaded, loaded);
        assert!(state.star_counts.candidate_regions >= state.star_counts.filtered && state.star_counts.filtered >= state.star_counts.projected);
        if frame == 1 { assert_eq!(state.star_counts, previous.unwrap()); }
        if frame == 3 { assert_eq!((state.star_counts.filtered, state.star_counts.projected), (0, 0)); }
        previous = Some(state.star_counts);
    }
    state.replace_catalog(astroterm::sky::prepare_owned_catalog(astroterm::catalog::Catalog::new(Vec::new(), Default::default(), Vec::new())).unwrap());
    assert_eq!(state.star_counts, StarCounts::default());
}
