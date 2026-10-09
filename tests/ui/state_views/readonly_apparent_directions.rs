use astroterm::state::ApparentDirections;
fn modify_stars(apparent: ApparentDirections<'_>) {
    apparent.regions().next().unwrap().1[0].x = 0.0;
}
fn modify_planets(apparent: ApparentDirections<'_>) {
    apparent.bodies().0[0].x = 0.0;
}
fn main() {}
