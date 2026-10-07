use astroterm::state::StellarResults;
fn modify_stellar_results(results: StellarResults<'_>) {
    results.samples()[0].1 = 0.0;
}
fn main() {}
