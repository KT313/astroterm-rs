//! Region versions and stable catalog identities across the complete cached frame pipeline.
use std::sync::Arc;
use astroterm::{astro::{J2000, Observer, Vector3}, cache::{CacheConfig, Group, GroupPolicy},
    catalog::{Catalog, ConstellationFigure, StarId, load_embedded_catalog},
    model::{FrameTime, ObservedRegion, ObservedSky, ProjectionViewport, SkyCatalog, SkyRegion, View, ViewCenter},
    state::{ObservationCache, ObserverPreparationCache, ProjectionCache, SimulationState, StarSelectionCache, StellarSimulationState},
    timing::StepTimes, projection, sky};

fn catalog() -> Arc<SkyCatalog> {
    let template = load_embedded_catalog().unwrap().stars[0].clone();
    let stars = (0..12).map(|index| {
        let mut star = template.clone();
        star.id = StarId(index + 1); star.hr = Some(index + 1); star.has_data = true;
        star.right_ascension = f64::from(index / 3) * 0.9 - 1.35 + f64::from(index % 3) * 0.001;
        star.declination = 0.0; star.magnitude = 3.0 + f64::from(index % 3) * 3.0;
        star.ra_motion = 0.0; star.ra_motion_cos_dec = 0.0; star.dec_motion = 0.0; star.space_motion = None;
        star
    }).collect();
    Arc::new(sky::prepare_owned_catalog(Catalog::new(stars, Default::default(), vec![
        ConstellationFigure { abbreviation: "Test", segments: vec![[1, 10]] },
    ])).unwrap().catalog)
}
struct Run {
    catalog: Arc<SkyCatalog>, solar: SimulationState, observer: ObserverPreparationCache,
    selection: StarSelectionCache, stellar: StellarSimulationState, observation: ObservationCache,
    projection: ProjectionCache, sky: ObservedSky,
}
impl Run {
    fn new(catalog: Arc<SkyCatalog>, config: CacheConfig) -> Self {
        let mut solar = SimulationState::default(); solar.configure_cache(&config);
        Self { sky: ObservedSky::new(catalog.clone()), catalog, solar,
            observer: ObserverPreparationCache::new(config.clone()), selection: StarSelectionCache::new(config.clone()),
            stellar: StellarSimulationState::new(config.clone()), observation: ObservationCache::new(config.clone()),
            projection: ProjectionCache::new(config) }
    }
    fn frame(&mut self, epoch: f64, region_ra: Option<f64>, threshold: f64, refract: bool, view: View, viewport: ProjectionViewport) -> Vec<ObservedRegion> {
        let mut times = StepTimes::default();
        let time = FrameTime { utc: epoch, ut1: epoch, tt: epoch };
        sky::begin_solar_system_frame(&mut self.solar, &mut self.observer, time, Observer::default(), &mut times).unwrap();
        let observer = sky::prepare_observer_inputs(&mut self.observer, &mut self.solar, time, Observer::default(), &mut times).unwrap();
        let region = region_ra.map_or(SkyRegion::All, |ra| SkyRegion::Cone {
            center: observer.inertial_to_horizon.apply(Vector3 { x: ra.cos(), y: ra.sin(), z: 0.0 }), radius: 0.04,
        });
        sky::select_cached_stars(&mut self.selection, &self.catalog, &observer, threshold, refract, region, &mut times);
        sky::simulate_stars(&mut self.stellar, self.selection.stars(), epoch, &mut times);
        let observed = sky::observe_cached_regions(&mut self.observation, self.stellar.results(self.selection.stars()),
            self.observer.bodies(&observer), &observer, threshold, refract, &mut self.sky, &mut times);
        let regions = observed.regions().to_vec();
        let completed = observed.sky().materialize(); // explicit reference snapshot; production keeps the borrowed view
        assert_eq!(regions.first().map_or(0, |r| r.start), 0);
        assert_eq!(regions.last().map_or(0, |r| r.end), observed.sky().stars.len());
        for pair in regions.windows(2) { assert_eq!(pair[0].end, pair[1].start); }
        projection::project_cached_regions(&mut self.projection, observed, &view, viewport, epoch, &mut times);
        let reference = projection::project_sky(&completed, &view, viewport);
        let mut expected = reference.view(&completed);
        let actual = projection::borrow_projected(&self.projection, observed.sky(), &view, viewport);
        let expected_stars: Vec<_> = regions.iter().filter(|r| r.start != r.end).flat_map(|region| {
            let rows = &completed.stars[region.start..region.end];
            expected.stars.iter().filter(move |star| rows.iter().any(|row| row.source_index == star.star.source_index))
        }).collect();
        assert_eq!(actual.stars.iter().collect::<Vec<_>>(), expected_stars);
        expected.stars = actual.stars;
        assert!(actual == expected, "projected geometry or metadata changed");
        self.sky = completed;
        regions
    }
}

#[test]
fn regional_pipeline_matches_bypass_through_selection_time_threshold_and_view_changes() {
    let catalog = catalog();
    let mut bypass = CacheConfig::default();
    for group in [Group::CandidateSelection, Group::WorkingSet, Group::StellarVisibility, Group::ApparentDirections, Group::Projection, Group::DrawOrder] {
        bypass.groups.insert(group, GroupPolicy { enabled: false, max_age_seconds: None });
    }
    let mut cached = Run::new(catalog.clone(), CacheConfig::default());
    let mut direct = Run::new(catalog, bypass); // both runs intentionally retain the same ten-day stellar samples
    for (n, (days, region, threshold, refract)) in [
        (0.0, Some(-0.45), 5.0, false), (0.0, None, 5.0, false), (0.0, Some(-0.45), 5.0, false),
        (0.0, Some(0.45), 10.0, true), (0.0, None, 2.0, true), (1.0, None, 10.0, true),
        (11.0, Some(-0.45), 5.0, false), (-1.0, None, 5.0, false),
    ].into_iter().enumerate() {
        let view = View { center: ViewCenter::Facing { azimuth: n as f64 * 0.3, tilt: 0.2 }, fov_degrees: if n % 2 == 0 { 180.0 } else { 100.0 }, ..View::default() };
        let viewport = ProjectionViewport { width: 80 + n, height: 40 + n };
        cached.frame(J2000 + days, region, threshold, refract, view, viewport);
        direct.frame(J2000 + days, region, threshold, refract, view, viewport);
        assert_eq!(cached.sky.stars, direct.sky.stars, "stellar output, frame {n}");
        assert_eq!(cached.sky.planets, direct.sky.planets, "body output, frame {n}");
        assert_eq!(cached.sky.moon, direct.sky.moon, "Moon output, frame {n}");
        assert!(cached.sky == direct.sky, "remaining observed metadata differs, frame {n}");
    }
}

#[test]
fn returning_to_a_region_keeps_its_independent_versions() {
    let mut run = Run::new(catalog(), CacheConfig::default());
    let view = View::default(); let viewport = ProjectionViewport { width: 80, height: 40 };
    let first = run.frame(J2000, Some(-0.45), 5.0, false, view, viewport);
    run.frame(J2000, Some(0.45), 5.0, false, view, viewport);
    let returned = run.frame(J2000, Some(-0.45), 5.0, false, view, viewport);
    assert_eq!(first, returned); // no other requested region may advance this region's dependency versions
}
