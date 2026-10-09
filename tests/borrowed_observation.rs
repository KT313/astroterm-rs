//! Production observation borrows original rows and final directions; only explicit reference snapshots own rows.
use std::sync::Arc;
use astroterm::{astro::{J2000, Observer}, cache::CacheConfig,
    model::{FrameTime, ObservedSky, SkyCatalog, SkyRegion, View, ProjectionViewport},
    state::{SimulationState, ObserverPreparationCache, StarSelectionCache, StellarSimulationState, ObservationCache, ProjectionCache, Tables},
    sky, projection, timing::StepTimes};

struct Run {
    catalog: Arc<SkyCatalog>, solar: SimulationState, observer: ObserverPreparationCache,
    selection: StarSelectionCache, stellar: StellarSimulationState, observation: ObservationCache,
    projection: ProjectionCache, summary: ObservedSky,
}
impl Run {
    fn new(config: CacheConfig, count: usize) -> Self {
        let mut source = astroterm::catalog::load_embedded_catalog().unwrap();
        source.stars.retain(|star| star.has_data); source.stars.truncate(count);
        source.constellations.clear();
        if count >= 2 { source.constellations.push(astroterm::catalog::ConstellationFigure { abbreviation: "Test", segments: vec![[source.stars[0].hr.unwrap(), source.stars[count - 1].hr.unwrap()]] }); }
        for (index, star) in source.stars.iter_mut().enumerate() {
            star.space_motion = None; star.ra_motion = 0.0; star.ra_motion_cos_dec = 0.0; star.dec_motion = 0.0;
            star.right_ascension = index as f64 * 0.35; star.declination = 0.1;
            star.magnitude = (index % 9) as f64;
        }
        let catalog = Arc::new(sky::prepare_owned_catalog(source).unwrap().catalog);
        let mut solar = SimulationState::default(); solar.configure_cache(&config);
        Self { summary: ObservedSky::new(catalog.clone()), catalog, solar,
            observer: ObserverPreparationCache::new(config.clone()), selection: StarSelectionCache::new(config.clone()),
            stellar: StellarSimulationState::new(config.clone()), observation: ObservationCache::new(config.clone()), projection: ProjectionCache::new(config) }
    }
    fn frame(&mut self, utc: f64, site: Observer, threshold: f64, refract: bool, cone: bool) -> (ObservedSky, StepTimes) {
        let mut times = StepTimes::with_trace(true);
        let time = FrameTime::from_utc(utc);
        sky::begin_solar_system_frame(&mut self.solar, &mut self.observer, time, site, &mut times).unwrap();
        let observer = sky::prepare_observer_inputs(&mut self.observer, &mut self.solar, time, site, &mut times).unwrap();
        let region = if cone { SkyRegion::Cone { center: observer.inertial_to_horizon.apply(self.catalog.stars.stored_direction(0)), radius: 0.2 } } else { SkyRegion::All };
        sky::select_cached_stars(&mut self.selection, &self.catalog, &observer, threshold, refract, region, &mut times);
        sky::simulate_stars(&mut self.stellar, self.selection.stars(), time.tt, &mut times);
        let observed = sky::observe_cached_regions(&mut self.observation, self.stellar.results(self.selection.stars()), self.observer.bodies(&observer), &observer, threshold, refract, &mut self.summary, &mut times);
        let view = observed.sky();
        let mut mixed = view.stars.iter();
        let expected_count = mixed.len();
        let mut consumed = 0;
        while mixed.len() > 0 {
            if consumed % 2 == 0 { assert!(mixed.next().is_some()); } else { assert!(mixed.next_back().is_some()); }
            consumed += 1;
        }
        assert_eq!(consumed, expected_count);
        assert!(mixed.next().is_none() && mixed.next_back().is_none());
        drop(mixed); // end the iterator borrow before inspecting the state owner
        assert!(view.stars.iter().map(|row| row.source_index).eq(view.stars.iter().rev().map(|row| row.source_index).collect::<Vec<_>>().into_iter().rev()));
        let snapshot = view.materialize(); // test/export only; production holds the borrowed view
        let mut expected = ObservedSky::new(self.catalog.clone());
        sky::observe_sky(&self.solar, &observer, threshold, refract, region, &mut expected, &mut StepTimes::default()).unwrap();
        assert_eq!(snapshot.stars, expected.stars);
        assert_eq!(snapshot.planets, expected.planets); assert_eq!(snapshot.moon, expected.moon);
        assert_eq!(snapshot.corrections, expected.corrections); assert_eq!(snapshot.selection, expected.selection);
        assert_eq!(snapshot.outside_accuracy_range, expected.outside_accuracy_range);
        let camera = View::default(); let viewport = ProjectionViewport { width: 100, height: 80 };
        projection::project_cached_regions(&mut self.projection, observed, &camera, viewport, time.tt, &mut times);
        let actual = projection::borrow_projected(&self.projection, view, &camera, viewport);
        let reference_data = projection::project_sky(&snapshot, &camera, viewport);
        let reference = reference_data.view(&snapshot);
        let mut actual_rows: Vec<_> = actual.stars.iter().map(|star| (star.star.source_index, star.cell, star.star.state.into_owned())).collect();
        let mut expected_rows: Vec<_> = reference.stars.iter().map(|star| (star.star.source_index, star.cell, star.star.state.into_owned())).collect();
        actual_rows.sort_by_key(|row| row.0); expected_rows.sort_by_key(|row| row.0);
        assert_eq!(actual_rows, expected_rows);
        assert_eq!(actual.constellations, reference.constellations);
        assert_eq!(actual.planets, reference.planets); assert_eq!(actual.moon, reference.moon);
        assert!(self.summary.stars.is_empty()); assert_eq!(self.summary.stars.capacity(), 0);
        (snapshot, times)
    }
}

#[test]
fn paused_frames_skip_all_bulk_observation_output_work() {
    let mut run = Run::new(CacheConfig::default(), 40);
    for refract in [false, true, false] {
        let (expected, _) = run.frame(J2000, Observer::default(), 6.0, refract, false);
        let (actual, times) = run.frame(J2000, Observer::default(), 6.0, refract, false);
        assert!(actual == expected, "paused owned snapshots differ");
        let forbidden = ["Corrected-star buffer construction", "Horizon rotation calculation", "Refraction calculation", "Direction work preparation", "Direction capture", "Direction restoration", "Observed output materialization"];
        assert!(!times.trace().unwrap().steps.iter().any(|step| forbidden.contains(&step.name)));
        let borrowed = run.observation.observed_view(run.stellar.results(run.selection.stars()), &run.summary);
        assert_eq!(borrowed.stars.len(), actual.stars.len());
    }
}

#[test]
fn borrowed_results_follow_site_time_membership_and_refraction_changes() {
    for config in [CacheConfig::default(), CacheConfig::disabled()] {
        let mut run = Run::new(config, 40);
        for (time, site, threshold, refract, cone) in [
            (J2000, Observer::default(), 20.0, false, false),
            (J2000, Observer::default(), 3.0, true, true),
            (J2000 + 0.01, Observer { latitude: 0.7, longitude: 1.1 }, 5.0, true, false),
            (J2000 - 0.01, Observer::default(), -20.0, false, false),
            (J2000, Observer::default(), 20.0, true, false),
        ] { run.frame(time, site, threshold, refract, cone); }
    }
    Run::new(CacheConfig::default(), 0).frame(J2000, Observer::default(), 5.0, true, false);
}

#[test]
fn completed_view_is_invalidated_and_direction_work_capacity_is_retained() {
    let mut run = Run::new(CacheConfig::disabled(), 40);
    for _ in 0..3 { run.frame(J2000, Observer::default(), 20.0, true, false); }
    let mut capacities = Vec::new();
    run.observation.visit_tables("observation", &mut |name, table, _| {
        if name.ends_with("horizontal_work") || name.ends_with("refraction_work") { capacities.push((name.to_owned(), table.bytes().reserved)); }
    });
    assert_eq!(capacities.len(), 2);
    run.frame(J2000, Observer::default(), 20.0, true, false);
    run.observation.visit_tables("observation", &mut |name, table, _| {
        if let Some((_, capacity)) = capacities.iter().find(|(path, _)| path == name) { assert_eq!(table.bytes().reserved, *capacity); }
    });
    run.observation.invalidate_region(astroterm::constants::CONSTELLATION_REGION);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.observation.observed_view(run.stellar.results(run.selection.stars()), &run.summary))).is_err());
}
