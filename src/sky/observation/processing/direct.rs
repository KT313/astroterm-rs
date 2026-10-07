//! Cache-free corrections over already simulated stars and sampled bodies.
use crate::model::{ObservedSky, ObserverState, SelectedStar, BodySamples, ObservedStar};
use crate::astro::Vector3;
use crate::timing::StepTimes;
use super::stages::*;
use crate::sky::refract_sky_positions;
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_direct_observation(working: &[SelectedStar], motion: &[(Vector3, f64)], singular: usize, bodies: BodySamples,
    observer: &ObserverState, magnitude_threshold: f64, refraction: bool, output: &mut ObservedSky, times: &mut StepTimes) {
    assert_eq!(working.len(), motion.len());
    output.stars.clear();
    output.stars.extend(working.iter().zip(motion).map(|(star, &(position, magnitude))| ObservedStar { source_index: star.source_index, drawable: star.drawable, position, magnitude }));
    output.runtime_singular_count = singular;
    let tt = observer.time.tt;
    times.measure("Current brightness", || {
        filter_current_magnitudes(magnitude_threshold, output)
    });

    times.measure("Correction selection", || {
        select_corrections(output);
    });

    // form observer-relative geometry before the direction-only corrections
    let (relative_moon, relative_sun) = times.measure("Observer subtraction", || {
        subtract_observer_position(bodies, observer, output)
    });
    times.measure("Moon illumination", || {
        update_moon_illumination(relative_moon, relative_sun, output)
    });
    times.measure("Aberration", || apply_sky_aberration(observer.state.velocity, output));
    times.measure("Horizon rotation", || {
        rotate_sky_to_horizon(observer.inertial_to_horizon, output)
    });

    // publish coverage and optionally refract the completed horizontal directions
    output.outside_accuracy_range = crate::astro::accuracy::needs_accuracy_warning(tt);
    output.refracted = false;
    if refraction && observer.atmosphere {
        times.measure("Refraction", || refract_sky_positions(output));
    }
}
