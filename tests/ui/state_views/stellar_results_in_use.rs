use astroterm::state::{SelectedStars, StellarSimulationState};
fn invalidate_while_reading_samples(stars: &mut StellarSimulationState, selected: SelectedStars<'_>) {
    let result = stars.results(selected);
    let mut samples = result.selected_samples();
    stars.invalidate_region(0);
    let _ = samples.next();
}
fn main() {}
