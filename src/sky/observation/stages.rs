//! Private passes owned by observe_sky_candidates. Its output buffer holds inertial directions during preparation;
//! only the completed horizontal sky escapes the coordinator. No pass evaluates an ephemeris or sees a camera.
use crate::state::{SimulationState};
use crate::model::ObserverState;
use super::{LIGHT_SPEED_AU_DAY, apply_aberration, apply_unit_aberration, body_id};
use crate::astro::{Matrix3, Vector3};
use crate::model::{ObservedSky, PlanetKind, SkyCatalog, SimulationError};

use crate::model::BodySamples;

pub(super) fn sample_body_states(
    simulation: &SimulationState,
    observer: &ObserverState,
) -> Result<BodySamples, SimulationError> {
    let planets = PlanetKind::ALL
        .map(|kind| crate::sky::evaluate_body(simulation, body_id(kind), observer.emission_tt[body_id(kind) as usize]))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let moon = crate::sky::evaluate_body(simulation, crate::astro::models::BodyId::Moon,
        observer.emission_tt[crate::astro::models::BodyId::Moon as usize])?;
    Ok(BodySamples { planets, moon })
}

pub(super) fn filter_brightness_candidates(
    catalog: &SkyCatalog,
    tt: f64,
    threshold: f64,
    candidates: Option<&[usize]>,
) -> Vec<usize> {
    let keys = catalog.stars.brightness_keys();
    if !crate::astro::COMPUTATIONAL_INTERVAL.contains(tt) {
        (0..catalog.stars.len()).collect()
    } else if let Some(indices) = candidates {
        indices
            .iter()
            .copied()
            .filter(|&i| i < keys.len() && f64::from(keys[i]) <= threshold)
            .collect()
    } else {
        (0..catalog.stars.len())
            .filter(|&i| f64::from(keys[i]) <= threshold)
            .collect()
    }
}

/// Preserve source-index ordering and candidate eligibility while including every constellation endpoint.
pub(super) fn include_constellation_endpoints(selected: Vec<usize>, output: &mut ObservedSky) {
    include_constellation_endpoints_with_times(selected, output, &mut crate::timing::StepTimes::default());
}

use crate::model::SelectedStar;

pub(super) fn include_constellation_endpoints_with_times(
    selected: Vec<usize>,
    output: &mut ObservedSky,
    times: &mut crate::timing::StepTimes,
) {
    let working = merge_constellation_endpoints(selected, output.catalog.endpoint_indices(), times);
    let fields = output.catalog.stars.borrow_observation_fields();
    output.stars.clear();
    output.stars.extend(
        working
            .into_iter()
            .map(|star| fields.create_observed_star(star.source_index, star.drawable)),
    );
}

pub(super) fn merge_constellation_endpoints(
    mut selected: Vec<usize>,
    endpoints: &[usize],
    times: &mut crate::timing::StepTimes,
) -> Vec<SelectedStar> {
    let input = selected.len();
    let before = times.inspect_memory(|| crate::timing::BufferShape::vector(&selected, crate::timing::IndexDomain::Catalog));
    times.measure("Candidate index sort and dedup", || {
        selected.sort_unstable();
        selected.dedup();
    });
    {
        use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent, Operation};
        if let Some(shape) = before { times.record_memory(times.last_memory_step(), || MemoryEvent::borrow(BufferId::ValidatedCandidates, Access::Writable, shape)); }
        times.record_memory(times.last_memory_step(), || MemoryEvent::operation(BufferId::ValidatedCandidates, Operation::Write,
            before, Some(BufferShape::vector(&selected, IndexDomain::Catalog)), None, None)); // sort/dedup writes depend on comparisons
    }
    times.describe("Candidate index sort and dedup", || {
        format!(
            "input indices={input}; duplicates removed={}; output indices={}",
            input - selected.len(),
            selected.len()
        )
    });
    let input_shape = times.inspect_memory(|| crate::timing::BufferShape::vector(&selected, crate::timing::IndexDomain::Catalog));
    let working = times.measure("Endpoint index merge", || {
        let mut working = Vec::with_capacity(selected.len() + endpoints.len());
        let mut candidates = selected.into_iter().peekable();
        let mut endpoints = endpoints.iter().copied().peekable();
        while candidates.peek().is_some() || endpoints.peek().is_some() {
            let index = candidates
                .peek()
                .copied()
                .unwrap_or(usize::MAX)
                .min(endpoints.peek().copied().unwrap_or(usize::MAX));
            let drawable = candidates.peek() == Some(&index);
            if drawable {
                candidates.next();
            }
            if endpoints.peek() == Some(&index) {
                endpoints.next();
            }
            working.push(SelectedStar {
                source_index: index,
                drawable,
            });
        }
        working
    });
    {
        use crate::timing::{Access, BufferId, BufferShape, IndexDomain, MemoryEvent};
        if let Some(shape) = input_shape { times.record_memory(times.last_memory_step(), || MemoryEvent::borrow(BufferId::ValidatedCandidates, Access::ReadOnly, shape)); }
        times.record_borrow(BufferId::CatalogEndpoints, Access::ReadOnly, || BufferShape::slice(endpoints, IndexDomain::Catalog));
        times.record_build(BufferId::WorkingStars, || BufferShape::vector(&working, IndexDomain::Working));
    }
    times.describe("Endpoint index merge", || {
        format!(
            "output selected indices/flags={}; element bytes={}; catalog metadata copied=0",
            working.len(),
            std::mem::size_of::<SelectedStar>()
        )
    });
    working
}

pub(super) fn evaluate_stellar_motion(tt: f64, output: &mut ObservedSky) {
    let years = crate::astro::models::stars::years_since_j2000(tt);
    output.runtime_singular_count = 0;
    let trajectories = output.catalog.stars.borrow_trajectory_fields();
    for observed in &mut output.stars {
        let motion = trajectories.motion(observed.source_index);
        let sample = motion.evaluate(years, observed.magnitude);
        observed.position = sample.direction;
        observed.magnitude = sample.magnitude;
        output.runtime_singular_count += usize::from(sample.used_singular_fallback);
    }
}

/// Endpoints remain available even when too faint to draw as stars.
pub(super) fn filter_current_magnitudes(threshold: f64, output: &mut ObservedSky) {
    output.magnitude_threshold = threshold;
    for star in &mut output.stars {
        star.drawable &= star.magnitude <= threshold;
    }
}

pub(super) fn subtract_observer_position(
    samples: BodySamples,
    observer: &ObserverState,
    output: &mut ObservedSky,
) -> (Vector3, Vector3) {
    let relative_moon = samples.moon.position - observer.state.position;
    let relative_sun = samples.planets[0].position - observer.state.position;
    for (planet, state) in output.planets.iter_mut().zip(samples.planets) {
        planet.position = state.position - observer.state.position;
    }
    output.moon.position = relative_moon;
    (relative_moon, relative_sun)
}

/// Phase uses un-aberrated observer-relative Sun/Moon vectors in the common inertial frame.
pub(super) fn update_moon_illumination(relative_moon: Vector3, relative_sun: Vector3, output: &mut ObservedSky) {
    output.moon.illumination = crate::sky::compute_moon_illumination(
        relative_moon,
        relative_sun,
        crate::astro::models::orientation::j2000_ecliptic_north(),
    );
    output.moon.phase = crate::sky::name_moon_phase(output.moon.illumination);
}

pub(super) fn apply_sky_aberration(velocity: Vector3, output: &mut ObservedSky) {
    let beta = velocity * (1.0 / LIGHT_SPEED_AU_DAY);
    for star in &mut output.stars {
        star.position = apply_unit_aberration(star.position, beta);
    }
    for planet in &mut output.planets {
        planet.position = apply_aberration(planet.position, velocity);
    }
    output.moon.position = apply_aberration(output.moon.position, velocity);
}

pub(super) fn rotate_sky_to_horizon(rotation: Matrix3, output: &mut ObservedSky) {
    for star in &mut output.stars {
        star.position = rotation.apply(star.position);
    }
    for planet in &mut output.planets {
        planet.position = rotation.apply(planet.position).normalized();
    }
    output.moon.position = rotation.apply(output.moon.position);
}

/// Select candidates needing directions with one walk through sorted constellation endpoints.
pub(super) fn select_correction_indices(
    sources: impl ExactSizeIterator<Item = usize>,
    drawable: &[bool],
    endpoints: &[usize],
) -> (Vec<usize>, crate::model::CorrectionStats) {
    let mut endpoints = endpoints.iter().copied().peekable();
    let mut indices = Vec::with_capacity(sources.len());
    let mut stats = crate::model::CorrectionStats {
        evaluated: sources.len(),
        ..Default::default()
    };
    for (index, (star, &drawable)) in sources.zip(drawable).enumerate() {
        if drawable {
            indices.push(index);
            continue;
        }
        while endpoints.peek().is_some_and(|&i| i < star) {
            endpoints.next();
        }
        if endpoints.peek() == Some(&star) {
            indices.push(index);
            stats.endpoint_only += 1;
        } else {
            stats.skipped += 1;
        }
    }
    (indices, stats)
}

/// Remove rejected non-endpoints before publication, so all returned positions receive the same corrections.
pub(super) fn select_corrections(output: &mut ObservedSky) {
    let flags: Vec<_> = output.stars.iter().map(|s| s.drawable).collect();
    let (indices, stats) = select_correction_indices(
        output.stars.iter().map(|s| s.source_index),
        &flags,
        output.catalog.endpoint_indices(),
    );
    output.corrections = stats;
    if stats.skipped == 0 {
        return;
    }
    let mut keep = indices.into_iter().peekable();
    let mut index = 0;
    output.stars.retain(|_| {
        let selected = keep.peek() == Some(&index);
        index += 1;
        if selected {
            keep.next();
        }
        selected
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_merge_preserves_source_order_and_membership_without_metadata() {
        let merged = merge_constellation_endpoints(vec![9, 1, 5, 1], &[0, 1, 7, 9, 12], &mut Default::default());
        assert_eq!(
            merged.iter().map(|s| (s.source_index, s.drawable)).collect::<Vec<_>>(),
            [(0, false), (1, true), (5, true), (7, false), (9, true), (12, false)]
        );
        assert!(std::mem::size_of::<SelectedStar>() <= 2 * std::mem::size_of::<usize>());
        assert_eq!(
            merge_constellation_endpoints(vec![], &[], &mut Default::default()),
            vec![]
        );
    }
}
