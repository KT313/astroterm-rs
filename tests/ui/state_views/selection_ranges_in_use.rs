use astroterm::state::StarSelectionCache;
fn invalidate_while_reading_ranges(cache: &mut StarSelectionCache) {
    let selected = cache.stars();
    let mut ranges = selected.ranges();
    cache.invalidate_view();
    let _ = ranges.next();
}
fn main() {}
