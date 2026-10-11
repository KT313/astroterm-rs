use astroterm::state::StellarResults;
fn modify_stellar_results(results: StellarResults<'_>) {
    results.selected_samples().next().unwrap().1.magnitude = 0;
}
fn main() {}
