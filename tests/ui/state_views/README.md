# Borrow contracts

`tests/state_views_compile.rs` runs these fixtures with trybuild. They use public model/state interfaces;
private stage views remain private. The passing fixture also executes a tiny headless frame with an empty catalog.
No terminal, filesystem dataset or large star list is needed.

| Fixture | Contract | Expected diagnostic |
|---|---|---|
| `readonly_indices.rs` | A borrowed catalog endpoint-index slice cannot be modified. | E0594 |
| `escaping_projection.rs` | A projected view cannot outlive its owned projection geometry. | E0515 |
| `overlapping_mutable_views.rs` | Two writable slices cannot overlap while both are still used. | E0499 |
| `observed_backing_in_use.rs` | Observation records cannot be changed while a projected rendering view borrows them. | E0502 |
| `projection_backing_in_use.rs` | Projection storage cannot be invalidated while a rendering view borrows it. | E0502 |
| `selection_ranges_in_use.rs` | Cached selection cannot be invalidated while its borrowed regional-range iterator remains in use. | E0502 |
| `stellar_results_in_use.rs` | Stellar storage cannot be invalidated while its selected-sample iterator remains in use. | E0502 |
| `readonly_apparent_directions.rs` | Borrowed apparent star and planet directions cannot be modified. | E0594 |
| `borrowed_observation_in_use.rs` | An observation cache cannot be invalidated while its borrowed final sky view remains in use. | E0502 |
| `disjoint_pipeline_fields.rs` | Source reads and separate projection, simulation and rendering writes can coexist. | Compiles and runs |

Snapshots were reviewed with `rustc 1.96.0 (ac68faa20 2026-05-25)`. On a compiler upgrade, inspect new diagnostics
before accepting snapshot changes: each failing case must still reject the intended ownership violation, rather
than an obsolete import, a private constructor or another unrelated error.

The selection and stellar-result fixtures also reject writes through `SelectedStars::rows()` and
`StellarResults::selected_samples()`. The passing fixture borrows selection while mutating stellar storage, then changes
the independent solar-system owner while the intrinsic star results remain borrowed.
