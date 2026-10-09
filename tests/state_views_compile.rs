/// These fixtures test public borrowed views; private stage buffers stay private.
#[test]
fn state_views_enforce_read_write_boundaries() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/state_views/readonly_indices.rs");
    cases.compile_fail("tests/ui/state_views/readonly_figures.rs");
    cases.compile_fail("tests/ui/state_views/escaping_projection.rs");
    cases.compile_fail("tests/ui/state_views/overlapping_mutable_views.rs");
    cases.compile_fail("tests/ui/state_views/observed_backing_in_use.rs");
    cases.compile_fail("tests/ui/state_views/projection_backing_in_use.rs");
    cases.compile_fail("tests/ui/state_views/readonly_selection.rs");
    cases.compile_fail("tests/ui/state_views/selection_ranges_in_use.rs");
    cases.compile_fail("tests/ui/state_views/readonly_stellar_results.rs");
    cases.compile_fail("tests/ui/state_views/stellar_results_in_use.rs");
    cases.compile_fail("tests/ui/state_views/readonly_apparent_directions.rs");
    cases.compile_fail("tests/ui/state_views/borrowed_observation_in_use.rs");
    cases.compile_fail("tests/ui/state_views/readonly_render_projection.rs");
    cases.pass("tests/ui/state_views/disjoint_pipeline_fields.rs");
}
