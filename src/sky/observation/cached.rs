//! Stateful observation owner. Each correction retains its own output; no corrected vector becomes a model input.
mod diagnostics;
mod stellar;
use super::{stages::*, *};
use crate::astro::models::stars::{StellarMotion, StellarSample, years_since_j2000};
use crate::cache::{Cache, CacheConfig, Group};
use crate::sky::{ObservedStar, SkyCatalog};
use std::{collections::HashMap, sync::Arc};
#[cfg(test)]
use stellar::qualify_stellar_span;

type Directions = (Vec<Vector3>, Vec<Vector3>, Vector3);
type ObserverKey = (FrameTime, Observer, [u64; 3], u64, u64);
type BodyKey = (ObserverState, u64, u64);

#[derive(Clone, PartialEq)]
struct CorrectionSelection {
    indices: Vec<usize>,
    stats: crate::sky::CorrectionStats,
}

#[derive(Default)]
pub struct ObservationCache {
    config: CacheConfig,
    catalog: Option<Arc<SkyCatalog>>,
    observer: Cache<ObserverKey, ObserverState>,
    light_time: Cache<(ObserverState, [u64; 3]), ObserverState>,
    region: Cache<(crate::sky::SkyRegion, ObserverState, bool), crate::sky::grid::SelectedRegion>,
    candidates: Cache<(u64, f64), (Vec<usize>, crate::sky::SelectionStats)>,
    selected: Cache<(u64, bool), Vec<usize>>,
    working: Cache<u64, Vec<SelectedStar>>,
    stellar: HashMap<usize, Cache<(), StellarSample>>,
    stellar_stats: crate::cache::CacheStats,
    motion: Cache<u64, (Vec<(Vector3, f64)>, usize)>,
    eligible: Cache<(u64, u64, f64), Vec<bool>>,
    corrections: Cache<(u64, u64), CorrectionSelection>,
    bodies: Cache<BodyKey, BodySamples>,
    relative: Cache<(u64, BodyState), (Vec<Vector3>, Vector3)>,
    illumination: Cache<(Vector3, Vector3), (crate::sky::MoonIllumination, crate::astro::MoonPhase)>,
    apparent: Cache<(u64, u64, u64, Vector3), Directions>,
    horizontal: Cache<(u64, Matrix3), Directions>,
    refracted: Cache<(u64, bool), Directions>,
}
impl ObservationCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn invalidate_view(&mut self) {
        self.region.invalidate();
    }
    pub fn prepare_observer(
        &mut self,
        simulation: &SimulationState,
        time: FrameTime,
        site: Observer,
    ) -> Result<ObserverState, SimulationError> {
        let key = (
            time,
            site,
            simulation.model_versions(),
            simulation.refresh_counts.planets,
            simulation.refresh_counts.orientation,
        );
        if self
            .observer
            .needs_refresh(&key, time.tt, None, self.config.allows(Group::ObserverState))
        {
            let observer = super::prepare_observer(simulation, time, site)?;
            self.observer.store(key, time.tt, 0.0, observer);
        }
        Ok(*self.observer.value())
    }
    pub fn prepare_light_time(
        &mut self,
        simulation: &mut SimulationState,
        observer: &mut ObserverState,
        times: &mut StepTimes,
    ) -> Result<(), SimulationError> {
        let key = (*observer, simulation.model_versions());
        // A disabled model family must still receive frame-local emission coverage on a paused frame.
        let enabled = [
            Group::SolarSystemObservation,
            Group::PlanetarySamples,
            Group::LunarSamples,
        ]
        .into_iter()
        .all(|g| self.config.allows(g));
        if self.light_time.needs_refresh(&key, observer.time.tt, None, enabled) {
            super::prepare_light_time_samples(simulation, observer, times)?;
            self.light_time.store(key, observer.time.tt, 0.0, *observer);
        } else {
            *observer = *self.light_time.value();
        }
        Ok(())
    }
    /// Resolve all fallible body dependencies first; correction passes then publish a complete sky.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        simulation: &SimulationState,
        observer: &ObserverState,
        threshold: f64,
        refraction: bool,
        region: crate::sky::SkyRegion,
        output: &mut ObservedSky,
        times: &mut StepTimes,
    ) -> Result<(), SimulationError> {
        if self
            .catalog
            .as_ref()
            .is_none_or(|catalog| !Arc::ptr_eq(catalog, &output.catalog))
        {
            let observer_cache = std::mem::take(&mut self.observer);
            let light_time_cache = std::mem::take(&mut self.light_time);
            *self = Self::new(self.config.clone());
            self.observer = observer_cache;
            self.light_time = light_time_cache;
            self.catalog = Some(output.catalog.clone());
        }
        let mut previous_reports = None;
        times.measure_diagnostics(|_| previous_reports = Some(self.reports()));
        let epoch = observer.time.tt;
        let enabled = |g| self.config.allows(g);

        // candidate membership is independent from intrinsic stellar cache lifetimes
        times.measure("Region filtering", || {
            self.region.get_or_update(
                (region, *observer, refraction),
                epoch,
                enabled(Group::CandidateSelection),
                || {
                    output
                        .catalog
                        .grid
                        .select_region(region, observer, refraction && observer.atmosphere)
                },
            );
        });
        times.measure("Brightness bounds", || {
            self.candidates.get_or_update(
                (self.region.generation, threshold),
                epoch,
                enabled(Group::CandidateSelection),
                || {
                    let mut indices = Vec::new();
                    let stats = output.catalog.grid.select_brightness(
                        &output.catalog.stars,
                        self.region.value(),
                        threshold,
                        &mut indices,
                    );
                    (indices, stats)
                },
            );
        });
        let body_key = (
            *observer,
            simulation.refresh_counts.planets,
            simulation.refresh_counts.moon,
        );
        times.measure("Body sampling", || -> Result<(), SimulationError> {
            if self
                .bodies
                .needs_refresh(&body_key, epoch, None, enabled(Group::SolarSystemObservation))
            {
                let bodies = sample_body_states(simulation, observer)?;
                self.bodies.store(body_key, epoch, 0.0, bodies);
            }
            Ok(())
        })?;

        output.selection = self.candidates.value().1;
        times.measure("Candidate validation", || {
            self.selected.get_or_update(
                (
                    self.candidates.generation,
                    crate::astro::COMPUTATIONAL_INTERVAL.contains(epoch),
                ),
                epoch,
                enabled(Group::WorkingSet),
                || filter_brightness_candidates(&output.catalog, epoch, threshold, Some(&self.candidates.value().0)),
            );
        });
        times.measure_steps("Constellation endpoints", |times| {
            let refresh = times.measure("Working-set cache decision", || {
                self.working
                    .needs_refresh(&self.selected.generation, epoch, None, enabled(Group::WorkingSet))
            });
            if refresh {
                let selected = times.measure("Selected index copy", || self.selected.value().clone());
                times.describe("Selected index copy", || {
                    format!(
                        "copied indices={}; bytes={}",
                        selected.len(),
                        selected.len() * std::mem::size_of::<usize>()
                    )
                });
                let working = merge_constellation_endpoints(selected, &output.catalog.endpoint_indices, times);
                times.measure("Working-set cache store", || {
                    self.working.store(self.selected.generation, epoch, 0.0, working)
                });
            }
        });
        times.measure_steps("Stellar motion", |times| {
            self.update_stellar_motion(output, epoch, times)
        });
        let enabled = |g| self.config.allows(g);
        times.measure("Current brightness", || {
            self.eligible.get_or_update(
                (self.working.generation, self.motion.generation, threshold),
                epoch,
                enabled(Group::StellarVisibility),
                || {
                    self.working
                        .value()
                        .iter()
                        .zip(&self.motion.value().0)
                        .map(|(star, &(_, magnitude))| star.drawable && magnitude <= threshold)
                        .collect()
                },
            );
            output.magnitude_threshold = threshold;
        });

        times.measure_steps("Correction selection", |times| {
            let key = (self.working.generation, self.eligible.generation);
            let refresh = times.measure("Correction cache decision", || {
                self.corrections
                    .needs_refresh(&key, epoch, None, enabled(Group::StellarVisibility))
            });
            if refresh {
                let (indices, stats) = times.measure("Correction index selection", || {
                    select_correction_indices(
                        self.working.value().iter().map(|s| s.source_index),
                        self.eligible.value(),
                        &output.catalog.endpoint_indices,
                    )
                });
                times.measure("Correction cache store", || {
                    self.corrections
                        .store(key, epoch, 0.0, CorrectionSelection { indices, stats })
                });
            }
            let selection = self.corrections.value();
            times.measure("Corrected-star buffer construction", || {
                let working = self.working.value();
                let drawable = self.eligible.value();
                let samples = &self.motion.value().0;
                output.stars.clear();
                output.stars.extend(selection.indices.iter().map(|&index| ObservedStar {
                    source_index: working[index].source_index,
                    drawable: drawable[index],
                    position: samples[index].0,
                    magnitude: samples[index].1,
                }));
                output.corrections = selection.stats;
            });
            times.describe("Corrected-star buffer construction", || {
                format!(
                    "output records={}; estimated record bytes={}; calculated state only; catalog metadata copied=0",
                    output.stars.len(),
                    output.stars.len() * std::mem::size_of::<ObservedStar>()
                )
            });
        });

        // each cache owns a distinct coordinate-space result
        times.measure("Observer subtraction", || {
            let relative = self.relative.get_or_update(
                (self.bodies.generation, observer.state),
                epoch,
                enabled(Group::SolarSystemGeometry),
                || {
                    subtract_observer_position(self.bodies.value().clone(), observer, output);
                    (
                        output.planets.iter().map(|p| p.position).collect(),
                        output.moon.position,
                    )
                },
            );
            for (planet, &position) in output.planets.iter_mut().zip(&relative.0) {
                planet.position = position;
            }
            output.moon.position = relative.1;
        });
        times.measure("Moon illumination", || {
            let value = self.illumination.get_or_update(
                (output.moon.position, output.sun().position),
                epoch,
                enabled(Group::SolarSystemGeometry),
                || {
                    update_moon_illumination(output.moon.position, output.sun().position, output);
                    (output.moon.illumination, output.moon.phase)
                },
            );
            (output.moon.illumination, output.moon.phase) = *value;
        });
        times.measure_steps("Aberration", |times| {
            let key = (
                self.motion.generation,
                self.relative.generation,
                self.corrections.generation,
                observer.state.velocity,
            );
            let refresh = times.measure("Apparent cache decision", || {
                self.apparent
                    .needs_refresh(&key, epoch, None, enabled(Group::ApparentDirections))
            });
            if refresh {
                times.measure("Aberration calculation", || {
                    apply_sky_aberration(observer.state.velocity, output)
                });
                let positions = times.measure("Direction capture", || capture_directions(output));
                times.measure("Direction cache store", || {
                    self.apparent.store(key, epoch, 0.0, positions)
                });
            } else {
                times.measure("Direction restoration", || {
                    restore_directions(output, self.apparent.value())
                });
            }
        });
        times.measure_steps("Horizon rotation", |times| {
            let key = (self.apparent.generation, observer.inertial_to_horizon);
            let refresh = times.measure("Horizontal cache decision", || {
                self.horizontal
                    .needs_refresh(&key, epoch, None, enabled(Group::HorizontalSky))
            });
            if refresh {
                times.measure("Horizon rotation calculation", || {
                    rotate_sky_to_horizon(observer.inertial_to_horizon, output)
                });
                let positions = times.measure("Direction capture", || capture_directions(output));
                times.measure("Direction cache store", || {
                    self.horizontal.store(key, epoch, 0.0, positions)
                });
            } else {
                times.measure("Direction restoration", || {
                    restore_directions(output, self.horizontal.value())
                });
            }
        });
        output.refracted = false;
        if refraction && observer.atmosphere {
            times.measure_steps("Refraction", |times| {
                let key = (self.horizontal.generation, true);
                let refresh = times.measure("Refraction cache decision", || {
                    self.refracted
                        .needs_refresh(&key, epoch, None, enabled(Group::Refraction))
                });
                if refresh {
                    times.measure("Refraction calculation", || refract_sky_positions(output));
                    let positions = times.measure("Direction capture", || capture_directions(output));
                    times.measure("Direction cache store", || {
                        self.refracted.store(key, epoch, 0.0, positions)
                    });
                } else {
                    times.measure("Direction restoration", || {
                        restore_directions(output, self.refracted.value())
                    });
                }
                output.refracted = true;
            });
        }
        output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(epoch);
        times.measure_diagnostics(|times| {
        self.describe_observation(output, threshold, times);
        if let Some(previous) = previous_reports {
            for (before, after) in previous.into_iter().zip(self.reports()).skip(2) {
                times.describe(after.name, || format!("cache hits={} refreshes={} bypasses={}; last refresh reason={:?}; stored TT={:?}; validity={} s", after.stats.hits - before.stats.hits, after.stats.refreshes - before.stats.refreshes, after.stats.bypasses - before.stats.bypasses, after.stats.last_reason, after.calculated_at, after.valid_seconds));
            }
        }
        });
        Ok(())
    }
    pub fn reports(&self) -> Vec<crate::cache::CacheReport> {
        vec![
            self.observer.report("Observer geometry"),
            self.light_time.report("Light-time sampling"),
            self.region.report("Region filtering"),
            self.candidates.report("Brightness bounds"),
            self.selected.report("Candidate validation"),
            self.working.report("Constellation endpoints"),
            self.motion.report("Stellar motion"),
            self.eligible.report("Current brightness"),
            self.corrections.report("Correction selection"),
            self.bodies.report("Body sampling"),
            self.relative.report("Observer subtraction"),
            self.illumination.report("Moon illumination"),
            self.apparent.report("Aberration"),
            self.horizontal.report("Horizon rotation"),
            self.refracted.report("Refraction"),
        ]
    }
    pub fn stellar_report(&self, index: usize) -> Option<crate::cache::CacheReport> {
        self.stellar.get(&index).map(|c| c.report("Stellar state"))
    }
    pub fn stats(&self) -> crate::cache::CacheStats {
        let mut total = crate::cache::CacheStats::default();
        for s in [
            self.observer.stats,
            self.light_time.stats,
            self.region.stats,
            self.candidates.stats,
            self.selected.stats,
            self.working.stats,
            self.motion.stats,
            self.eligible.stats,
            self.corrections.stats,
            self.bodies.stats,
            self.relative.stats,
            self.illumination.stats,
            self.apparent.stats,
            self.horizontal.stats,
            self.refracted.stats,
        ]
        .into_iter()
        .chain([self.stellar_stats])
        {
            total.hits += s.hits;
            total.refreshes += s.refreshes;
            total.bypasses += s.bypasses;
        }
        total
    }
}

fn capture_directions(sky: &ObservedSky) -> Directions {
    (
        sky.stars.iter().map(|s| s.position).collect(),
        sky.planets.iter().map(|p| p.position).collect(),
        sky.moon.position,
    )
}
fn restore_directions(sky: &mut ObservedSky, directions: &Directions) {
    for (star, &p) in sky.stars.iter_mut().zip(&directions.0) {
        star.position = p;
    }
    for (planet, &p) in sky.planets.iter_mut().zip(&directions.1) {
        planet.position = p;
    }
    sky.moon.position = directions.2;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stellar_hold_bound_covers_forward_reverse_and_fast_motion() {
        let epoch = crate::astro::J2000;
        for speed in [0.0, 0.01, 10.0, 10000.0] {
            let motion = StellarMotion {
                u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
                w: Vector3 {
                    x: 0.0,
                    y: speed,
                    z: 0.0,
                },
                distance_pc: None,
            };
            let sample = motion.evaluate(0.0, 5.0);
            let span = qualify_stellar_span(motion, sample, epoch, 5.0, 360.0);
            for fraction in [-1.0, -0.3, 0.0, 0.4, 1.0] {
                let direct = motion.evaluate(years_since_j2000(epoch + span * fraction / 86400.0), 5.0);
                let error = sample
                    .direction
                    .cross(direct.direction)
                    .length()
                    .atan2(sample.direction.dot(direct.direction));
                assert!(error.to_degrees() * 3600.0 <= 0.1);
                assert_eq!(direct.magnitude, sample.magnitude);
            }
        }
    }
    #[test]
    fn variable_brightness_and_out_of_range_states_use_exact_epochs() {
        let motion = StellarMotion {
            u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w: Vector3 {
                x: -0.01,
                y: 0.001,
                z: 0.0,
            },
            distance_pc: Some(1.0),
        };
        let sample = motion.evaluate(0.0, 5.0);
        assert_eq!(
            qualify_stellar_span(motion, sample, crate::astro::J2000, 5.0, 360.0),
            0.0
        );
        assert_eq!(
            qualify_stellar_span(motion, sample, crate::astro::COMPUTATIONAL_INTERVAL.end_tt, 5.0, 360.0),
            0.0
        );
    }
}
