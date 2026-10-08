//! Region, brightness bounds, validation and endpoint merge in execution order.
use crate::state::StarSelectionCache;
use crate::model::{SkyCatalog, ObserverState, SkyRegion};
use crate::timing::StepTimes;
use std::sync::Arc;
use super::caching::*;

pub fn select_cached_stars(storage: &mut StarSelectionCache, catalog: &Arc<SkyCatalog>, observer: &ObserverState,
    threshold: f64, refraction: bool, region: SkyRegion, times: &mut StepTimes) {
    if storage.catalog.as_ref().is_none_or(|previous| !Arc::ptr_eq(previous, catalog)) {
        *storage = StarSelectionCache::new(storage.config.clone());
        storage.catalog = Some(catalog.clone());
    }
    let previous = times.trace().map(|_| storage.reports());
    let epoch = observer.time.tt;
    update_region_filtering(&mut storage.region, &storage.config, epoch, refraction, region, observer, &catalog.grid, times);
    update_brightness_bounds(&mut storage.candidates, &storage.region, &storage.config, epoch, threshold, catalog, times);
    update_candidate_validation(&mut storage.selected, &storage.candidates, &storage.config, epoch, times);
    update_constellation_endpoints(&mut storage.working, &storage.selected, &storage.config, epoch, catalog.endpoint_indices(), times);
    storage.requested_epoch = Some(epoch);
    describe_selection(storage, catalog, threshold, times);
    crate::sky::describe_cache_reports(previous, || storage.reports(), times);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{astro::{J2000, COMPUTATIONAL_INTERVAL, Matrix3, Observer, Vector3, models::BodyState}, cache::CacheConfig, model::FrameTime};

    #[test]
    fn trusted_selection_matches_generic_validation_across_regions_thresholds_and_epochs() {
        let mut source = crate::catalog::load_embedded_catalog().unwrap();
        source.stars.retain(|star| star.has_data);
        source.stars.truncate(32);
        source.constellations = vec![crate::catalog::ConstellationFigure {
            abbreviation: "Test", segments: vec![[source.stars[0].hr.unwrap(), source.stars[1].hr.unwrap()]],
        }];
        let catalog = Arc::new(crate::sky::prepare_catalog(&source).unwrap().catalog);
        assert_eq!(catalog.endpoint_indices().len(), 2);
        let malformed = [usize::MAX, catalog.stars.len(), 0, 0];
        assert_eq!(super::super::processing::filter_brightness_candidates(&catalog, J2000, 20.0, Some(&malformed)), vec![0, 0]);
        assert_eq!(super::super::processing::filter_brightness_candidates(&catalog, COMPUTATIONAL_INTERVAL.end_tt, -20.0, Some(&malformed)), (0..catalog.stars.len()).collect::<Vec<_>>());
        for config in [CacheConfig::default(), CacheConfig::disabled()] {
            let mut cache = StarSelectionCache::new(config.clone());
            for epoch in [J2000, COMPUTATIONAL_INTERVAL.end_tt + 1.0, J2000] {
                let observer = crate::sky::compose_observer_state(FrameTime { utc: epoch, ut1: epoch, tt: epoch }, Observer::default(), BodyState::default(), Matrix3::IDENTITY, BodyState::default(), false);
                for region in [SkyRegion::All, SkyRegion::Cone { center: Vector3 { x: 1.0, y: 0.0, z: 0.0 }, radius: 0.1 }] {
                    for threshold in [-20.0, 3.0, 5.0, 20.0] {
                        let mut times = StepTimes::with_trace(true);
                        select_cached_stars(&mut cache, &catalog, &observer, threshold, false, region, &mut times);
                        let selected = super::super::processing::filter_brightness_candidates(&catalog, epoch, threshold, Some(&cache.candidates.value().0));
                        let expected = super::super::processing::merge_constellation_endpoints(selected.clone(), catalog.endpoint_indices(), &mut Default::default());
                        assert_eq!(cache.selected.value(), &selected);
                        assert_eq!(cache.working.value(), &expected);
                        assert!(selected.windows(2).all(|pair| pair[0] < pair[1]));
                        assert!(!times.trace().unwrap().steps.iter().any(|step| ["Selected index copy", "Candidate index sort and dedup"].contains(&step.name)));
                        let generations = (cache.region.generation, cache.candidates.generation, cache.selected.generation, cache.working.generation);
                        let refreshes = cache.stats().refreshes;
                        select_cached_stars(&mut cache, &catalog, &observer, threshold, false, region, &mut times);
                        assert_eq!((cache.region.generation, cache.candidates.generation, cache.selected.generation, cache.working.generation), generations);
                        assert_eq!(cache.stats().refreshes - refreshes, if config.enabled { 0 } else { 4 });
                        assert_eq!(cache.working.value(), &expected);
                    }
                }
            }
        }
    }
}
