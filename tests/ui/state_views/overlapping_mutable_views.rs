use astroterm::state::Caches;

fn update_overlapping_stars(run: &mut Caches) {
    let first = run.sky.stars.as_mut_slice();
    let second = run.sky.stars.as_mut_slice(); // the first writable view still covers these same rows
    first[0].drawable = false;
    second[0].drawable = true;
}

fn main() {}
