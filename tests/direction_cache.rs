//! Refresh calculations already populate directions; only cache hits restore saved coordinate-space results.
use astroterm::state::{ObservationCache, SimulationState};
use astroterm::astro::{J2000, Observer};
use astroterm::cache::CacheConfig;
use astroterm::catalog::load_embedded_catalog;
use astroterm::model::{ObservedSky, SkyCatalog};
use astroterm::model::projection::View;
use astroterm::model::simulation::FrameTime;
use astroterm::sky::{observe_sky, update_simulation};
use astroterm::timing::StepTimes;
use std::sync::Arc;

struct Pipeline {
    simulation: SimulationState,
    observation: ObservationCache,
    sky: ObservedSky,
}

impl Pipeline {
    fn new(catalog: Arc<SkyCatalog>, config: CacheConfig) -> Self {
        let mut simulation = SimulationState::default();
        simulation.configure_cache(&config);
        Self {
            simulation,
            observation: ObservationCache::new(config),
            sky: ObservedSky::new(catalog),
        }
    }

    fn frame(&mut self, site: Observer, refraction: bool) -> StepTimes {
        let time = FrameTime::from_utc(J2000);
        let mut times = StepTimes::with_trace(true);
        times.begin_frame();
        self.simulation.begin_frame();
        update_simulation(&mut self.simulation, time, &[], &mut times).unwrap();
        let mut observer = astroterm::sky::prepare_cached_observer(&mut self.observation, &self.simulation, time, site).unwrap();
        astroterm::sky::prepare_cached_light_time(&mut self.observation, &mut self.simulation, &mut observer, &mut times)
            .unwrap();
        astroterm::sky::observe_cached_sky(&mut self.observation, &self.simulation,
                &observer,
                5.0,
                refraction,
                astroterm::projection::select_view_region(&View::default()),
                &mut self.sky,
                &mut times)
            .unwrap();
        // Use the same sampled models so sample-holding differences cannot mask correction errors.
        let mut reference = ObservedSky::new(self.sky.catalog.clone());
        observe_sky(
            &self.simulation,
            &observer,
            5.0,
            refraction,
            astroterm::projection::select_view_region(&View::default()),
            &mut reference,
            &mut StepTimes::default(),
        )
        .unwrap();
        assert_matching_sky(&self.sky, &reference);
        times
    }
}

fn assert_direction_steps(times: &StepTimes, stage: &str, calculation: &str, refresh: bool) {
    let steps = &times.trace().unwrap().steps;
    let index = steps.iter().position(|step| step.name == stage).unwrap();
    let children = steps[index + 1..]
        .iter()
        .take_while(|step| step.depth > steps[index].depth)
        .map(|step| step.name)
        .collect::<Vec<_>>();
    assert_eq!(children.contains(&calculation), refresh, "{stage}: {children:?}");
    assert_eq!(
        children.contains(&"Direction capture"),
        refresh,
        "{stage}: {children:?}"
    );
    assert_eq!(
        children.contains(&"Direction restoration"),
        !refresh,
        "{stage}: {children:?}"
    );
}

fn assert_matching_sky(actual: &ObservedSky, expected: &ObservedSky) {
    assert_eq!(actual.stars.len(), expected.stars.len());
    for (actual, expected) in actual.stars.iter().zip(&expected.stars) {
        assert_eq!(actual, expected);
    }
    assert_eq!(actual.planets, expected.planets);
    assert_eq!(actual.moon, expected.moon);
    assert_eq!(actual.selection, expected.selection);
    assert_eq!(actual.corrections, expected.corrections);
}

#[test]
fn direction_refreshes_skip_restoration_and_paused_hits_restore_exact_results() {
    let catalog = Arc::new(astroterm::sky::prepare_owned_catalog(load_embedded_catalog().unwrap()));
    let mut cached = Pipeline::new(catalog.clone(), CacheConfig::default());
    let first_site = Observer::default();
    let next_site = Observer {
        latitude: 0.5,
        longitude: 1.0,
    };

    // Verify both initial calculation and dependency changes, each followed by paused reuse.
    for site in [first_site, next_site] {
        for refresh in [true, false] {
            let times = cached.frame(site, true);
            for (stage, calculation) in [
                ("Aberration", "Aberration calculation"),
                ("Horizon rotation", "Horizon rotation calculation"),
                ("Refraction", "Refraction calculation"),
            ] {
                assert_direction_steps(&times, stage, calculation, refresh);
            }
        }
    }

    // Toggling atmospheric rendering must select the correct cached coordinate-space result.
    for refraction in [false, true, false] {
        cached.frame(next_site, refraction); // changing refraction can also change candidate membership
        let times = cached.frame(next_site, refraction);
        assert_direction_steps(&times, "Aberration", "Aberration calculation", false);
        assert_direction_steps(&times, "Horizon rotation", "Horizon rotation calculation", false);
        if refraction {
            assert_direction_steps(&times, "Refraction", "Refraction calculation", false);
        } else {
            assert!(
                !times
                    .trace()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|step| step.name == "Refraction")
            );
        }
    }
}

#[test]
fn bypass_always_calculates_without_restoring_even_when_paused() {
    let catalog = Arc::new(astroterm::sky::prepare_owned_catalog(load_embedded_catalog().unwrap()));
    let mut bypassed = Pipeline::new(catalog, CacheConfig::disabled());
    let mut expected = None;
    for _ in 0..3 {
        let times = bypassed.frame(Observer::default(), true);
        if let Some(expected) = &expected {
            assert_matching_sky(&bypassed.sky, expected);
        } else {
            expected = Some(bypassed.sky.clone());
        }
        for (stage, calculation) in [
            ("Aberration", "Aberration calculation"),
            ("Horizon rotation", "Horizon rotation calculation"),
            ("Refraction", "Refraction calculation"),
        ] {
            assert_direction_steps(&times, stage, calculation, true);
        }
    }
    assert_eq!(bypassed.observation.stats().hits, 0);
}
