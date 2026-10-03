//! Independently refreshed planetary, lunar and orientation samples. No observer or camera lives here.
//! Linear intervals control interpolation error only, not the underlying ephemerides' astronomical accuracy.
//! Bounded samples cover reception and per-body emission epochs; extra disjoint requests fail explicitly.

use crate::astro::models::{
    BodyId, BodyState, moons::evaluate_moon, orientation::compute_slow_orientation, planets::evaluate_planets,
};
use crate::astro::{COMPUTATIONAL_INTERVAL, Matrix3};
use crate::timing::StepTimes;
use std::fmt;

/// UTC input approximates UT1; TT includes the Espenak–Meeus estimate of ΔT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    pub utc: f64,
    pub ut1: f64,
    pub tt: f64,
}
impl FrameTime {
    pub fn from_utc(utc: f64) -> Self {
        Self {
            utc,
            ut1: utc,
            tt: crate::astro::ut1_to_tt(utc),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFamily {
    Planets,
    Moon,
    Orientation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateRequest {
    pub body: BodyId,
    pub tt: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimulationError {
    InvalidTime,
    MissingCoverage { family: ModelFamily, tt: f64 },
    TooManyEpochs(ModelFamily),
    NonFiniteState(ModelFamily),
}
impl fmt::Display for SimulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "simulation state unavailable: {self:?}")
    }
}
impl std::error::Error for SimulationError {}

/// Per-family interpolation limits, measured against direct model evaluation. These do not include the physical
/// theory's error. Lunar limits are parent-relative; common-frame composition adds the parent's error. For the
/// Earth observer, the same-time parent position cancels before the direction is formed.
#[derive(Clone, Copy, Debug)]
pub struct InterpolationLimits {
    pub position_au: f64,
    pub velocity_au_day: f64,
    pub orientation_arcseconds: f64,
}
pub const PLANET_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: 3e-8,
    velocity_au_day: 5e-5,
    orientation_arcseconds: 0.0,
};
pub const MOON_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: 1e-9,
    velocity_au_day: 1e-6,
    orientation_arcseconds: 0.0,
};
pub const ORIENTATION_LIMITS: InterpolationLimits = InterpolationLimits {
    position_au: 0.0,
    velocity_au_day: 0.0,
    orientation_arcseconds: 0.2,
};

/// Sampled interpolation policy, days either side of the sample. Outside the computational interval only an exact
/// sample is accepted. Bounds are qualified by the cadence sweep, separate from physical accuracy targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachePolicy {
    pub planets_days: f64,
    pub moon_days: f64,
    pub orientation_days: f64,
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            planets_days: 30.0 / 86400.0,
            moon_days: 12.0 / 86400.0,
            orientation_days: 60.0 / 86400.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefreshCounts {
    pub planets: u64,
    pub moon: u64,
    pub orientation: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Sample<T> {
    epoch: f64,
    half_span: f64,
    value: T,
}
impl<T> Sample<T> {
    fn covers(&self, tt: f64) -> bool {
        tt == self.epoch || (COMPUTATIONAL_INTERVAL.contains(tt) && (tt - self.epoch).abs() <= self.half_span)
    }
}

/// Concrete family caches, with parent-relative lunar storage. Replacing a lunar theory/settings only clears
/// lunar samples. Orientation model changes also invalidate the Moon's mean-of-date adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationState {
    planets: Vec<Sample<[BodyState; 9]>>,
    moon: Vec<Sample<BodyState>>,
    orientation: Vec<Sample<Matrix3>>,
    policy: CachePolicy,
    pub refresh_counts: RefreshCounts,
    versions: [u64; 3],
}
impl SimulationState {
    /// Apply validated policies once at startup. A zero span keeps exact within-frame samples only.
    pub fn configure_cache(&mut self, config: &crate::cache::CacheConfig) {
        use crate::cache::Group;
        self.policy = CachePolicy {
            planets_days: config.age_seconds(Group::PlanetarySamples) / 86400.0,
            moon_days: config.age_seconds(Group::LunarSamples) / 86400.0,
            orientation_days: config.age_seconds(Group::SlowOrientation) / 86400.0,
        };
        self.planets.clear();
        self.moon.clear();
        self.orientation.clear();
    }
    /// Clear bypassed families once per frame, not between reception and emission requests.
    pub fn begin_frame(&mut self) {
        if self.policy.planets_days == 0.0 {
            self.planets.clear();
        }
        if self.policy.moon_days == 0.0 {
            self.moon.clear();
        }
        if self.policy.orientation_days == 0.0 {
            self.orientation.clear();
        }
    }
    pub fn model_versions(&self) -> [u64; 3] {
        self.versions
    }

    /// Direct per-frame evaluation, used as the reference for cache qualification.
    pub fn exact() -> Self {
        Self {
            policy: CachePolicy {
                planets_days: 0.0,
                moon_days: 0.0,
                orientation_days: 0.0,
            },
            ..Self::default()
        }
    }

    pub fn set_model_version(&mut self, family: ModelFamily, version: u64) {
        let index = family as usize;
        if self.versions[index] == version {
            return;
        }
        self.versions[index] = version;
        match family {
            ModelFamily::Planets => self.planets.clear(),
            ModelFamily::Moon => self.moon.clear(),
            ModelFamily::Orientation => {
                self.orientation.clear();
                self.moon.clear();
            }
        }
    }

    /// Read-only same-time evaluation; missing coverage is a coordinator error, never a hidden ephemeris call.
    pub fn evaluate_body(&self, body: BodyId, tt: f64) -> Result<BodyState, SimulationError> {
        if body == BodyId::Moon {
            let sample = find_sample(&self.moon, tt, ModelFamily::Moon)?;
            let relative = sample.value.evaluate(tt - sample.epoch);
            return Ok(relative.add_parent(self.evaluate_body(BodyId::Earth, tt)?));
        }
        let sample = find_sample(&self.planets, tt, ModelFamily::Planets)?;
        Ok(sample.value[body as usize].evaluate(tt - sample.epoch))
    }

    pub fn evaluate_orientation(&self, tt: f64) -> Result<Matrix3, SimulationError> {
        Ok(find_sample(&self.orientation, tt, ModelFamily::Orientation)?.value)
    }
}

fn find_sample<T>(samples: &[Sample<T>], tt: f64, family: ModelFamily) -> Result<&Sample<T>, SimulationError> {
    samples
        .iter()
        .find(|sample| sample.covers(tt))
        .ok_or(SimulationError::MissingCoverage { family, tt })
}

/// Ensure reception and explicitly requested emission epochs are covered, refreshing only missing families.
/// Observation preparation supplies observer-dependent light-time requests; all sample mutation stays here.
pub fn update_simulation(
    state: &mut SimulationState,
    time: FrameTime,
    requests: &[StateRequest],
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    if ![time.utc, time.ut1, time.tt].into_iter().all(f64::is_finite) || requests.iter().any(|r| !r.tt.is_finite()) {
        return Err(SimulationError::InvalidTime);
    }
    let mut planet_epochs = vec![time.tt];
    let mut moon_epochs = vec![time.tt];
    for request in requests {
        planet_epochs.push(request.tt); // the Moon also needs its parent at emission, never at reception
        if request.body == BodyId::Moon {
            moon_epochs.push(request.tt);
        }
    }
    times.measure("Planet samples", || {
        prepare_samples(
            &mut state.planets,
            &planet_epochs,
            state.policy.planets_days,
            ModelFamily::Planets,
            &mut state.refresh_counts.planets,
            |tt| {
                let values = evaluate_planets(tt);
                if values.iter().all(is_finite_state) {
                    Ok(values)
                } else {
                    Err(SimulationError::NonFiniteState(ModelFamily::Planets))
                }
            },
        )
    })?;
    times.measure("Lunar samples", || {
        prepare_samples(
            &mut state.moon,
            &moon_epochs,
            state.policy.moon_days,
            ModelFamily::Moon,
            &mut state.refresh_counts.moon,
            |tt| {
                let value = evaluate_moon(tt);
                if is_finite_state(&value) {
                    Ok(value)
                } else {
                    Err(SimulationError::NonFiniteState(ModelFamily::Moon))
                }
            },
        )
    })?;
    times.measure("Orientation samples", || {
        prepare_samples(
            &mut state.orientation,
            &[time.tt],
            state.policy.orientation_days,
            ModelFamily::Orientation,
            &mut state.refresh_counts.orientation,
            |tt| {
                let value = compute_slow_orientation(tt);
                if value.0.iter().flatten().all(|v| v.is_finite()) {
                    Ok(value)
                } else {
                    Err(SimulationError::NonFiniteState(ModelFamily::Orientation))
                }
            },
        )
    })?;
    Ok(())
}

fn is_finite_state(state: &BodyState) -> bool {
    [
        state.position.x,
        state.position.y,
        state.position.z,
        state.velocity.x,
        state.velocity.y,
        state.velocity.z,
    ]
    .into_iter()
    .all(f64::is_finite)
}

/// Prepare required coverage first, then retain a bounded recent working set for subsequent emission requests.
fn prepare_samples<T: Clone>(
    samples: &mut Vec<Sample<T>>,
    epochs: &[f64],
    half_span: f64,
    family: ModelFamily,
    counter: &mut u64,
    evaluate: impl Fn(f64) -> Result<T, SimulationError>,
) -> Result<(), SimulationError> {
    let maximum_samples = match family {
        ModelFamily::Planets => BodyId::PLANETS.len() + 2, // reception, planetary emissions, and the lunar parent
        ModelFamily::Moon => 2,
        ModelFamily::Orientation => 1,
    };
    let mut prepared: Vec<Sample<T>> = Vec::with_capacity(2);
    for &tt in epochs {
        if prepared.iter().any(|sample| sample.covers(tt)) {
            continue;
        }
        if prepared.len() == maximum_samples {
            return Err(SimulationError::TooManyEpochs(family));
        }
        if let Some(sample) = samples.iter().find(|sample| sample.covers(tt)) {
            prepared.push(sample.clone());
        } else {
            let span = if COMPUTATIONAL_INTERVAL.contains(tt) {
                half_span
                    .min(tt - COMPUTATIONAL_INTERVAL.start_tt)
                    .min(COMPUTATIONAL_INTERVAL.end_tt - tt)
            } else {
                0.0
            };
            prepared.push(Sample {
                epoch: tt,
                half_span: span,
                value: evaluate(tt)?,
            });
            *counter += 1;
        }
    }
    // retain a bounded working history so reception-only preparation does not evict emission coverage
    for sample in samples.iter() {
        if prepared.len() >= maximum_samples * 2 {
            break;
        }
        if !prepared.iter().any(|p| p.epoch == sample.epoch) {
            prepared.push(sample.clone());
        }
    }
    *samples = prepared;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::{J2000, Observer, Vector3};
    use crate::canvas::Canvas;
    use crate::catalog::load_embedded_catalog;
    use crate::projection::{View, Viewport, project_sky};
    use crate::scene::{RenderOptions, draw_sky_scene};
    use crate::sky::{Sky, observe_sky, prepare_observer};

    fn linear_parent(tt: f64) -> Result<BodyState, SimulationError> {
        Ok(BodyState {
            position: Vector3 {
                x: 1.0 + (tt - J2000) * 0.01,
                y: 0.0,
                z: 0.0,
            },
            velocity: Vector3 {
                x: 0.01,
                y: 0.0,
                z: 0.0,
            },
        })
    }
    fn linear_moon(tt: f64) -> Result<BodyState, SimulationError> {
        Ok(BodyState {
            position: Vector3 {
                x: 0.001,
                y: (tt - J2000) * 0.0001,
                z: 0.0,
            },
            velocity: Vector3 {
                x: 0.0,
                y: 0.0001,
                z: 0.0,
            },
        })
    }

    #[test]
    fn retention_is_bounded_and_reception_keeps_emission_coverage() {
        let mut samples = Vec::new();
        let mut count = 0;
        let epoch = J2000;
        prepare_samples(
            &mut samples,
            &[epoch, epoch - 0.1],
            1e-4,
            ModelFamily::Planets,
            &mut count,
            linear_parent,
        )
        .unwrap();
        let original = count;
        for _ in 0..5 {
            prepare_samples(
                &mut samples,
                &[epoch],
                1e-4,
                ModelFamily::Planets,
                &mut count,
                linear_parent,
            )
            .unwrap();
            prepare_samples(
                &mut samples,
                &[epoch, epoch - 0.1],
                1e-4,
                ModelFamily::Planets,
                &mut count,
                linear_parent,
            )
            .unwrap();
        }
        assert_eq!(count, original);
        for i in 1..200 {
            let tt = epoch + (i as f64 * 0.2) * if i % 2 == 0 { 1.0 } else { -1.0 };
            prepare_samples(
                &mut samples,
                &[tt, tt - 0.1],
                1e-4,
                ModelFamily::Planets,
                &mut count,
                linear_parent,
            )
            .unwrap();
            assert!(samples.len() <= 22);
            assert!(find_sample(&samples, tt, ModelFamily::Planets).is_ok());
            assert!(find_sample(&samples, tt - 0.1, ModelFamily::Planets).is_ok());
        }
    }

    #[test]
    fn independent_synthetic_intervals_compose_at_the_requested_epoch() {
        let (mut parent, mut moon) = (Vec::new(), Vec::new());
        let (mut pc, mut mc) = (0, 0);
        for delta in [0.0, 0.125, 0.375, 0.625, 1.125] {
            let tt = J2000 + delta;
            prepare_samples(&mut parent, &[tt], 1.0, ModelFamily::Planets, &mut pc, linear_parent).unwrap();
            prepare_samples(&mut moon, &[tt], 0.25, ModelFamily::Moon, &mut mc, linear_moon).unwrap();
            let p = find_sample(&parent, tt, ModelFamily::Planets).unwrap();
            let m = find_sample(&moon, tt, ModelFamily::Moon).unwrap();
            let composed = m
                .value
                .evaluate(tt - m.epoch)
                .add_parent(p.value.evaluate(tt - p.epoch));
            let direct = linear_moon(tt).unwrap().add_parent(linear_parent(tt).unwrap());
            assert!((composed.position - direct.position).length() < 1e-14);
        }
        assert_eq!((pc, mc), (2, 3));
    }

    fn lunar_a(_: f64) -> BodyState {
        BodyState {
            position: Vector3 {
                x: 0.002,
                y: 0.001,
                z: 0.0005,
            },
            ..BodyState::default()
        }
    }
    fn lunar_b(_: f64) -> BodyState {
        BodyState {
            position: Vector3 {
                x: -0.001,
                y: 0.002,
                z: -0.0005,
            },
            ..BodyState::default()
        }
    }

    #[test]
    fn two_concrete_lunar_evaluators_share_the_pipeline_without_changing_planets() {
        let mut simulation = SimulationState::default();
        let time = FrameTime::from_utc(J2000);
        update_simulation(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
        let planets = simulation.planets.clone();
        let observer = prepare_observer(&simulation, time, Observer::default()).unwrap();
        let mut sky = Sky::from_catalog(&load_embedded_catalog().unwrap());
        let options = RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            constellations: true,
            grid: false,
            magnitude_threshold: 5.0,
            label_threshold: 0.25,
            dynamic_names: true,
        };
        let mut observed_positions = Vec::new();
        for evaluator in [lunar_a as fn(f64) -> BodyState, lunar_b] {
            simulation.moon = vec![Sample {
                epoch: time.tt,
                half_span: 0.1,
                value: evaluator(time.tt),
            }];
            observe_sky(
                &simulation,
                &observer,
                5.0,
                false,
                crate::sky::SkyRegion::All,
                &mut sky,
                &mut StepTimes::default(),
            )
            .unwrap();
            observed_positions.push(sky.moon.position);
            let projected = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
            draw_sky_scene(&mut Canvas::new(41, 81), &options, &projected);
            assert_eq!(simulation.planets, planets);
        }
        assert_ne!(observed_positions[0], observed_positions[1]);
    }
}
