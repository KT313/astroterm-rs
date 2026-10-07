//! Cache-free stellar reference evaluation over a selected set of immutable catalog rows.
use crate::model::{StellarFields, SelectedStar};
use crate::astro::Vector3;
pub(crate) fn simulate_stars_direct(catalog: StellarFields<'_>, selected: &[SelectedStar], tt: f64) -> (Vec<(Vector3, f64)>, usize) {
    let years = crate::astro::models::stars::years_since_j2000(tt);
    let trajectories = &catalog;
    let mut singular = 0;
    let values = selected.iter().map(|star| {
        let sample = trajectories.motion(star.source_index).evaluate(years, catalog.magnitude(star.source_index));
        singular += usize::from(sample.used_singular_fallback);
        (sample.direction, sample.magnitude)
    }).collect();
    (values, singular)
}
