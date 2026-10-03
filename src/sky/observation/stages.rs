//! Private passes owned by observe_sky_candidates. Its output buffer holds inertial directions during preparation;
//! only the completed horizontal sky escapes the coordinator. No pass evaluates an ephemeris or sees a camera.
use super::{LIGHT_SPEED_AU_DAY, ObserverState, apply_aberration, apply_unit_aberration, body_id};
use crate::astro::{Matrix3, Vector3};
use crate::sky::{ObservedSky, PlanetKind, SimulationError, SimulationState, SkyCatalog};

#[derive(Clone, PartialEq)]
pub(super) struct BodySamples {
    planets: Vec<crate::astro::models::BodyState>,
    moon: crate::astro::models::BodyState,
}

pub(super) fn sample_body_states(
    simulation: &SimulationState,
    observer: &ObserverState,
) -> Result<BodySamples, SimulationError> {
    let planets = PlanetKind::ALL
        .map(|kind| simulation.evaluate_body(body_id(kind), observer.emission_tt[body_id(kind) as usize]))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let moon = simulation.evaluate_body(
        crate::astro::models::BodyId::Moon,
        observer.emission_tt[crate::astro::models::BodyId::Moon as usize],
    )?;
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
pub(super) fn include_constellation_endpoints(mut selected: Vec<usize>, output: &mut ObservedSky) {
    selected.sort_unstable();
    selected.dedup();
    let mut candidates = selected.into_iter().peekable();
    let mut endpoints = output.catalog.endpoint_indices.iter().copied().peekable();
    let fields = output.catalog.stars.borrow_observation_fields();
    output.stars.clear();
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
        output.stars.push(fields.create_observed_star(index, drawable));
    }
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
    output.moon.phase = output.moon.illumination.named_phase();
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
    stars: &[crate::sky::ObservedStar],
    drawable: &[bool],
    endpoints: &[usize],
) -> (Vec<usize>, crate::sky::CorrectionStats) {
    let mut endpoints = endpoints.iter().copied().peekable();
    let mut indices = Vec::with_capacity(stars.len());
    let mut stats = crate::sky::CorrectionStats {
        evaluated: stars.len(),
        ..Default::default()
    };
    for (index, (star, &drawable)) in stars.iter().zip(drawable).enumerate() {
        if drawable {
            indices.push(index);
            continue;
        }
        while endpoints.peek().is_some_and(|&i| i < star.source_index) {
            endpoints.next();
        }
        if endpoints.peek() == Some(&star.source_index) {
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
    let (indices, stats) = select_correction_indices(&output.stars, &flags, &output.catalog.endpoint_indices);
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
