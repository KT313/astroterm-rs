//! Private passes owned by observe_sky_candidates. Its output buffer holds inertial directions during preparation;
//! only the completed horizontal sky escapes the coordinator. No pass evaluates an ephemeris or sees a camera.
use super::{ObserverState, apply_aberration, body_id};
use crate::astro::{Matrix3, Vector3};
use crate::sky::{ObservedSky, ObservedStar, PlanetKind, SimulationError, SimulationState, SkyCatalog};

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
    if !crate::astro::COMPUTATIONAL_INTERVAL.contains(tt) {
        (0..catalog.stars.len()).collect()
    } else if let Some(indices) = candidates {
        indices
            .iter()
            .copied()
            .filter(|&i| i < catalog.stars.len() && catalog.stars.brightness_key(i) <= threshold)
            .collect()
    } else {
        (0..catalog.stars.len())
            .filter(|&i| catalog.stars.brightness_key(i) <= threshold)
            .collect()
    }
}

/// Preserve source-index ordering and candidate eligibility while including every constellation endpoint.
pub(super) fn include_constellation_endpoints(mut selected: Vec<usize>, output: &mut ObservedSky) {
    selected.sort_unstable();
    selected.dedup();
    let mut candidates = selected.into_iter().peekable();
    let mut endpoints = output.catalog.endpoint_indices.iter().copied().peekable();
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
        let star = output.catalog.stars.get(index);
        let mut observed = ObservedStar::from_star(&star, index, Vector3::default());
        observed.drawable = drawable;
        output.stars.push(observed);
    }
}

pub(super) fn evaluate_stellar_motion(tt: f64, output: &mut ObservedSky) {
    let years = crate::astro::models::stars::years_since_j2000(tt);
    output.runtime_singular_count = 0;
    for observed in &mut output.stars {
        let motion = output.catalog.stars.motion(observed.source_index);
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
    for star in &mut output.stars {
        star.position = apply_aberration(star.position, velocity);
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
