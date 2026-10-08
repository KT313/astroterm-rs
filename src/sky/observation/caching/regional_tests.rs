//! Region reuse, stable row identities and exact correction results across changing requests.
use crate::{astro::{J2000, Matrix3, Vector3}, cache::{CacheConfig, Group, GroupPolicy}, model::{FrameTime, ObservedSky, ObserverState, SkyCatalog, SkyRegion},
    state::SimulationState, test_pipeline::PipelineCache, timing::StepTimes};
use std::sync::Arc;

fn catalog() -> Arc<SkyCatalog> {
    let mut source = crate::catalog::load_embedded_catalog().unwrap();
    source.stars.retain(|star| star.has_data);
    source.stars.truncate(5);
    source.constellations = vec![crate::catalog::ConstellationFigure {
        abbreviation: "Test", segments: vec![[source.stars[3].hr.unwrap(), source.stars[4].hr.unwrap()]],
    }];
    for (index, star) in source.stars.iter_mut().enumerate() {
        star.right_ascension = index as f64 * std::f64::consts::FRAC_PI_2;
        star.declination = 0.0;
        star.ra_motion = 0.0;
        star.ra_motion_cos_dec = 0.0;
        star.dec_motion = 0.0;
        star.space_motion = None;
        star.magnitude = 4.0 + index as f64;
    }
    Arc::new(crate::sky::prepare_owned_catalog(source).unwrap().catalog)
}

fn prepare() -> (SimulationState, ObserverState) {
    let mut simulation = SimulationState::default();
    let time = FrameTime::from_utc(J2000);
    crate::sky::update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
    let mut observer = crate::sky::prepare_observation(&mut simulation, time, Default::default()).unwrap();
    observer.inertial_to_horizon = Matrix3::IDENTITY;
    (simulation, observer)
}

fn cone(x: f64, y: f64) -> SkyRegion { SkyRegion::Cone { center: Vector3 { x, y, z: 0.0 }, radius: 0.1 } }

fn frame(cache: &mut PipelineCache, simulation: &SimulationState, observer: &ObserverState, threshold: f64, region: SkyRegion, output: &mut ObservedSky) {
    let mut times = StepTimes::default();
    crate::sky::prepare_cached_bodies(&mut cache.observer, simulation, observer, &mut times).unwrap();
    crate::sky::select_cached_stars(&mut cache.selection, &output.catalog, observer, threshold, true, region, &mut times);
    crate::sky::simulate_stars(&mut cache.stars, cache.selection.stars(), observer.time.tt, &mut times);
    let result = crate::sky::observe_cached_regions(&mut cache.observation, cache.stars.results(cache.selection.stars()), cache.observer.bodies(observer), observer, threshold, true, output, &mut times);
    assert_eq!(result.regions().len(), cache.selection.stars().regions().len());
    let mut cursor = 0;
    for descriptor in result.regions() {
        assert_eq!(descriptor.start, cursor);
        cursor = descriptor.end;
        let rows = &result.sky().stars[descriptor.start..descriptor.end];
        let offsets = &result.sky().catalog.grid.offsets;
        assert!(rows.iter().all(|row| row.source_index >= offsets[descriptor.region] && row.source_index < offsets[descriptor.region + 1]));
        assert!(rows.windows(2).all(|pair| pair[0].source_index < pair[1].source_index));
    }
    assert_eq!(cursor, result.sky().stars.len());
    for descriptor in &cache.observation.regional_output {
        let reports = cache.observation.region_reports(descriptor.region).unwrap();
        assert_eq!(descriptor.selection_generation, reports[1].generation);
        assert_eq!(descriptor.apparent_generation, reports[2].generation);
    }
}

#[test]
fn overlapping_views_reuse_regions_and_preserve_unrequested_corrections() {
    let catalog = catalog();
    let (simulation, observer) = prepare();
    let mut cache = PipelineCache::default();
    let mut output = ObservedSky::new(catalog);
    frame(&mut cache, &simulation, &observer, 20.0, cone(1.0, 0.0), &mut output);
    let original = output.clone();
    let ordinary = cache.observation.regional_output.iter().find(|r| r.start != r.end && r.region != crate::constants::CONSTELLATION_REGION).unwrap().region;
    let versions = cache.observation.region_reports(ordinary).unwrap().map(|r| r.generation);
    assert!(cache.observation.regional_output.iter().any(|r| r.start == r.end));
    frame(&mut cache, &simulation, &observer, 20.0, SkyRegion::All, &mut output);
    assert!(cache.observation.region_reports(ordinary).unwrap().iter().all(|r| r.stats.refreshes == 1));
    let reports = cache.observation.region_reports(ordinary).unwrap();
    frame(&mut cache, &simulation, &observer, 20.0, cone(0.0, 1.0), &mut output);
    assert_eq!(cache.observation.region_reports(ordinary).unwrap(), reports);
    frame(&mut cache, &simulation, &observer, 20.0, cone(1.0, 0.0), &mut output);
    assert_eq!(output, original);
    assert_eq!(cache.observation.region_reports(ordinary).unwrap().map(|r| r.generation), versions);
    assert!(cache.observation.region_reports(ordinary).unwrap().iter().all(|r| r.stats.refreshes == 1));
    assert!(cache.observation.region_reports(crate::constants::CONSTELLATION_REGION).unwrap().iter().all(|r| r.stats.refreshes == 1));
    assert_eq!(cache.observation.body_apparent.stats.refreshes, 1);
    cache.observation.invalidate_region(ordinary);
    frame(&mut cache, &simulation, &observer, 20.0, cone(0.0, 1.0), &mut output);
    assert!(cache.observation.region_reports(ordinary).unwrap().iter().all(|r| r.has_been_invalidated));
    frame(&mut cache, &simulation, &observer, 20.0, cone(1.0, 0.0), &mut output);
    assert_eq!(output, original);
    assert_eq!(cache.observation.region_reports(ordinary).unwrap().map(|r| r.generation), versions);
    assert!(cache.observation.region_reports(ordinary).unwrap().iter().all(|r| r.stats.refreshes == 2));

}

#[test]
fn regional_thresholds_velocity_and_invalidation_match_uncached_results() {
    let catalog = catalog();
    let (simulation, mut observer) = prepare();
    let mut cache = PipelineCache::default();
    let mut reference = PipelineCache::new(CacheConfig::disabled());
    let mut output = ObservedSky::new(catalog.clone());
    let mut expected = ObservedSky::new(catalog);
    let endpoint = crate::constants::CONSTELLATION_REGION;
    for (index, threshold) in [20.0, -20.0, 20.0, 20.0, 20.0].into_iter().enumerate() {
        if index == 3 { observer.state.velocity.x += 0.001; }
        if index == 4 { cache.observation.invalidate_region(endpoint); }
        frame(&mut cache, &simulation, &observer, threshold, SkyRegion::All, &mut output);
        frame(&mut reference, &simulation, &observer, threshold, SkyRegion::All, &mut expected);
        assert_eq!(output, expected);
        let endpoint_rows = &cache.observation.regions[endpoint].corrections.value().0;
        assert_eq!(endpoint_rows.len(), 2);
        assert!(endpoint_rows.iter().all(|row| row.drawable == (threshold > 0.0)));
    }
    let reports = cache.observation.region_reports(endpoint).unwrap();
    assert_eq!(reports[0].stats.refreshes, 4); // velocity alone does not alter brightness
    assert_eq!(reports[1].stats.refreshes, 4);
    assert_eq!(reports[2].stats.refreshes, 5);
    assert_eq!(reports[2].generation, 2); // threshold only changes drawing flags; same endpoint directions remain reusable
}

#[test]
fn each_regional_group_honors_bypass_without_changing_results() {
    let catalog = catalog();
    let (simulation, observer) = prepare();
    for group in [None, Some(Group::StellarVisibility), Some(Group::ApparentDirections)] {
        let mut config = CacheConfig::default();
        if let Some(group) = group { config.groups.insert(group, GroupPolicy { enabled: false, max_age_seconds: None }); }
        else { config.enabled = false; }
        let mut cache = PipelineCache::new(config);
        let mut output = ObservedSky::new(catalog.clone());
        frame(&mut cache, &simulation, &observer, 20.0, cone(1.0, 0.0), &mut output);
        let expected = output.clone();
        frame(&mut cache, &simulation, &observer, 20.0, cone(1.0, 0.0), &mut output);
        assert_eq!(output, expected);
        let reports = cache.observation.region_reports(crate::constants::CONSTELLATION_REGION).unwrap();
        assert_eq!(reports[0].stats.bypasses, if group != Some(Group::ApparentDirections) { 2 } else { 0 });
        assert_eq!(reports[1].stats.bypasses, if group != Some(Group::ApparentDirections) { 2 } else { 0 });
        assert_eq!(reports[2].stats.bypasses, if group != Some(Group::StellarVisibility) { 2 } else { 0 });
        assert!(cache.observation.regions.iter().any(|region| region.eligible.stored().is_none()));
    }
}
