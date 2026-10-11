//! Viewer-dependent passes over completed stellar and solar-system inputs. No model sampling or camera access.
use crate::model::ObserverState;
use super::{apply_aberration, apply_unit_aberration};
use crate::astro::LIGHT_SPEED_AU_DAY;
use crate::astro::{Matrix3, Vector3};
use crate::model::ObservedSky;

use crate::model::BodySamples;

/// Endpoints remain available even when too faint to draw as stars.
pub(super) fn filter_current_magnitudes(threshold: f64, output: &mut ObservedSky) {
    output.magnitude_threshold = threshold;
    for star in &mut output.stars {
        star.drawable &= star.magnitude <= threshold;
    }
}

pub(super) fn subtract_observer_position(
    samples: &BodySamples,
    observer: &ObserverState,
    output: &mut ObservedSky,
) -> (Vector3, Vector3) {
    let relative_moon = samples.moon.position - observer.state.position;
    let relative_sun = samples.planets[0].position - observer.state.position;
    for (planet, state) in output.planets.iter_mut().zip(&samples.planets) {
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
