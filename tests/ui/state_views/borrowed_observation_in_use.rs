use astroterm::{model::ObservedSky, state::{ObservationCache, StellarResults}};
fn invalidate_while_borrowed(cache: &mut ObservationCache, stars: StellarResults<'_>, summary: &ObservedSky) {
    let observed = cache.observed_view(stars, summary);
    cache.invalidate_region(0);
    let _ = observed.stars.get(0);
}
fn main() {}
