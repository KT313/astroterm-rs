use astroterm::state::SelectedStars;
fn modify_selection(selection: SelectedStars<'_>) {
    selection.rows()[0].drawable = false;
}
fn main() {}
