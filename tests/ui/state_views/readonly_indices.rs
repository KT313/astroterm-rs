use astroterm::model::SkyCatalog;

fn change_endpoint(catalog: &SkyCatalog) {
    let endpoints: &[usize] = &catalog.endpoint_indices; // catalog indices are borrowed read-only
    endpoints[0] = 0;
}

fn main() {}
