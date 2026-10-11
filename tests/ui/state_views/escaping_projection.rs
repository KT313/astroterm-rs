use astroterm::model::{ObservedSky, ProjectedSky, ProjectionData};

fn return_view_of_dropped_storage(observed: &ObservedSky, backing: ProjectionData) -> ProjectedSky<'_> {
    backing.view(observed) // the returned view would outlive this function's owned geometry
}

fn main() {}
