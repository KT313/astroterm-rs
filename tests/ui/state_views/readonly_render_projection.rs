use astroterm::model::RenderProjection;
fn edit_trusted_geometry(projected: &mut RenderProjection<'_>) {
    projected.sky().fov_degrees = 20.0;
}
fn main() {}
