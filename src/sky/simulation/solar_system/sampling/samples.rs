//! Independently refreshed planetary, lunar and orientation samples. No observer or camera lives here.
//! Linear intervals control interpolation error only, not the underlying ephemerides' astronomical accuracy.
//! Bounded samples cover reception and per-body emission epochs; extra disjoint requests fail explicitly.

use crate::state::SimulationState;
use crate::astro::models::{
    BodyId, BodyState, moons::evaluate_moon, orientation::compute_slow_orientation, planets::evaluate_planets,
};
use crate::astro::COMPUTATIONAL_INTERVAL;
use crate::timing::{StepTimes, BufferId, BufferShape, IndexDomain};
use super::memory::record_sample_family;

use crate::model::{ModelFamily, SimulationError, Sample};

pub(super) fn refresh_planet_samples(state: &mut SimulationState, epochs: &[f64], before: crate::model::RefreshCounts, times: &mut StepTimes) -> Result<bool, SimulationError> {
    let memory_before = times.inspect_memory(|| (BufferShape::vector(&state.planets, IndexDomain::ModelSamples), BufferShape::vector(&state.planet_work, IndexDomain::ModelSamples)));
    let result = times.measure("Planet samples", || {
        prepare_samples(
            &mut state.planets,
            &mut state.planet_work,
            epochs,
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
    });
    record_sample_family(times, (BufferId::PlanetSamples, BufferId::PlanetSampleWork), memory_before, (&state.planets, &state.planet_work), state.refresh_counts.planets - before.planets, result.as_ref().ok().copied());
    let rebuilt = result?;
    times.describe("Planet samples", || format!("requested epochs={}; new sample blocks={}; retained blocks={}; half-span={} days; each evaluation supplies all planetary states", epochs.len(), state.refresh_counts.planets - before.planets, state.planets.len(), state.policy.planets_days));
    Ok(rebuilt)
}

pub(super) fn refresh_lunar_samples(state: &mut SimulationState, epochs: &[f64], before: crate::model::RefreshCounts, times: &mut StepTimes) -> Result<bool, SimulationError> {
    let memory_before = times.inspect_memory(|| (BufferShape::vector(&state.moon, IndexDomain::ModelSamples), BufferShape::vector(&state.moon_work, IndexDomain::ModelSamples)));
    let result = times.measure("Lunar samples", || {
        prepare_samples(
            &mut state.moon,
            &mut state.moon_work,
            epochs,
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
    });
    record_sample_family(times, (BufferId::LunarSamples, BufferId::LunarSampleWork), memory_before, (&state.moon, &state.moon_work), state.refresh_counts.moon - before.moon, result.as_ref().ok().copied());
    let rebuilt = result?;
    times.describe("Lunar samples", || {
        format!(
            "requested epochs={}; new sample blocks={}; retained blocks={}; half-span={} days",
            epochs.len(),
            state.refresh_counts.moon - before.moon,
            state.moon.len(),
            state.policy.moon_days
        )
    });
    Ok(rebuilt)
}

pub(super) fn refresh_orientation_samples(state: &mut SimulationState, tt: f64, before: crate::model::RefreshCounts, times: &mut StepTimes) -> Result<bool, SimulationError> {
    let memory_before = times.inspect_memory(|| (BufferShape::vector(&state.orientation, IndexDomain::ModelSamples), BufferShape::vector(&state.orientation_work, IndexDomain::ModelSamples)));
    let result = times.measure("Orientation samples", || {
        prepare_samples(
            &mut state.orientation,
            &mut state.orientation_work,
            &[tt],
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
    });
    record_sample_family(times, (BufferId::OrientationSamples, BufferId::OrientationSampleWork), memory_before, (&state.orientation, &state.orientation_work), state.refresh_counts.orientation - before.orientation, result.as_ref().ok().copied());
    let rebuilt = result?;
    times.describe("Orientation samples", || format!("requested epochs=1; new sample blocks={}; retained blocks={}; half-span={} days; output slow orientation matrices", state.refresh_counts.orientation - before.orientation, state.orientation.len(), state.policy.orientation_days));
    Ok(rebuilt)
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
    prepared: &mut Vec<Sample<T>>,
    epochs: &[f64],
    half_span: f64,
    family: ModelFamily,
    counter: &mut u64,
    evaluate: impl Fn(f64) -> Result<T, SimulationError>,
) -> Result<bool, SimulationError> {
    let maximum_samples = match family {
        ModelFamily::Planets => BodyId::PLANETS.len() + 2, // reception, planetary emissions, and the lunar parent
        ModelFamily::Moon => 2,
        ModelFamily::Orientation => 1,
    };
    if sample_list_unchanged(samples, epochs, maximum_samples) { return Ok(false); }
    prepared.clear();                                      // failed work from a previous attempt is never published
    prepared.reserve(maximum_samples * 2);                 // retain enough capacity for requests and bounded history
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
    std::mem::swap(samples, prepared);                     // publish by ownership transfer, not by copying sample contents
    prepared.clear();                                      // keep the old allocation for the next preparation
    Ok(true)
}


/// Required samples already lead the retained list in exactly the order preparation would produce.
fn sample_list_unchanged<T>(samples: &[Sample<T>], epochs: &[f64], maximum: usize) -> bool {
    let mut required = 0;
    for &tt in epochs {
        if samples[..required].iter().any(|sample| sample.covers(tt)) { continue; }
        if required == maximum || !samples.get(required).is_some_and(|sample| sample.covers(tt)) { return false; }
        required += 1;
    }
    true
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FrameTime;
    use super::super::find_sample;
    use crate::sky::update_solar_system;
    use crate::astro::{J2000, Observer, Vector3};
    use crate::canvas::Canvas;
    use crate::catalog::load_embedded_catalog;
    use crate::model::{ProjectionViewport as Viewport, View};
    use crate::projection::project_sky;
    use crate::model::RenderOptions;
    use crate::scene::draw_sky_scene;
    use crate::sky::{observe_sky, prepare_observer};

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
        let mut work = Vec::new();
        let mut count = 0;
        let epoch = J2000;
        prepare_samples(
            &mut samples, &mut work,
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
                &mut samples, &mut work,
                &[epoch],
                1e-4,
                ModelFamily::Planets,
                &mut count,
                linear_parent,
            )
            .unwrap();
            prepare_samples(
                &mut samples, &mut work,
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
                &mut samples, &mut work,
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
        let (mut parent_work, mut moon_work) = (Vec::new(), Vec::new());
        let (mut pc, mut mc) = (0, 0);
        for delta in [0.0, 0.125, 0.375, 0.625, 1.125] {
            let tt = J2000 + delta;
            prepare_samples(&mut parent, &mut parent_work, &[tt], 1.0, ModelFamily::Planets, &mut pc, linear_parent).unwrap();
            prepare_samples(&mut moon, &mut moon_work, &[tt], 0.25, ModelFamily::Moon, &mut mc, linear_moon).unwrap();
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

    #[test]
    fn work_capacity_is_reused_and_failure_keeps_previous_samples() {
        let (mut samples, mut work) = (Vec::new(), Vec::new());
        let mut count = 0;
        for delta in [0.0, 1.0] {
            prepare_samples(&mut samples, &mut work, &[J2000 + delta], 0.0, ModelFamily::Planets, &mut count, linear_parent).unwrap();
        }
        let mut allocations = [samples.as_ptr(), work.as_ptr()];
        allocations.sort();
        let capacities = (samples.capacity(), work.capacity());
        for delta in 2..12 {
            prepare_samples(&mut samples, &mut work, &[J2000 + f64::from(delta)], 0.0, ModelFamily::Planets, &mut count, linear_parent).unwrap();
            let mut actual = [samples.as_ptr(), work.as_ptr()]; actual.sort();
            assert_eq!(actual, allocations); // both allocations stay alive throughout; no freed-address inference
            assert_eq!((samples.capacity(), work.capacity()), capacities);
            assert!(work.is_empty());
        }
        let previous = samples.clone();
        let pointer = samples.as_ptr();
        let result = prepare_samples(&mut samples, &mut work, &[J2000 + 20.0, J2000 + 21.0], 0.0, ModelFamily::Planets, &mut count,
            |tt| if tt == J2000 + 21.0 { Err(SimulationError::NonFiniteState(ModelFamily::Planets)) } else { linear_parent(tt) });
        assert!(result.is_err());
        assert_eq!(samples, previous);
        assert_eq!(samples.as_ptr(), pointer);
        assert_eq!(work.len(), 1); // partial work remains inspectable, never published
        prepare_samples(&mut samples, &mut work, &[J2000 + 22.0], 0.0, ModelFamily::Planets, &mut count, linear_parent).unwrap();
        assert_eq!(samples[0].epoch, J2000 + 22.0);
        assert!(!samples.iter().any(|s| s.epoch == J2000 + 20.0));
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
        update_solar_system(&mut simulation, time, &[], &mut StepTimes::default()).unwrap();
        let planets = simulation.planets.clone();
        let observer = prepare_observer(&simulation, time, Observer::default()).unwrap();
        let mut sky = crate::sky::create_sky_from_catalog(&load_embedded_catalog().unwrap()).unwrap();
        let options = RenderOptions {
            unicode: true,
            braille: true,
            color: true,
            constellations: true,
            grid: false,
            magnitude_threshold: 5.0,
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
                crate::model::SkyRegion::All,
                &mut sky,
                &mut StepTimes::default(),
            )
            .unwrap();
            observed_positions.push(sky.moon.position);
            let projected_data = project_sky(&sky, &View::default(), Viewport { height: 41, width: 81 });
            let projected = projected_data.view(&sky);
            draw_sky_scene(&mut Canvas::new(41, 81), &options, &projected);
            assert_eq!(simulation.planets, planets);
        }
        assert_ne!(observed_positions[0], observed_positions[1]);
    }
}
