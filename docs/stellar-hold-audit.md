# Offline stellar hold accuracy

This opt-in test compares directions held by `SimulationCache` with fresh evaluations of the same motion
model at the requested date. It does not change the speed groups, TTLs, calculation epochs or rendering.
It reports each group's sample count, number of violations and worst error, with the internal star ID,
region, cached calculation time and requested time (TT Julian dates). Constellation stars are audited separately.

Run the small synthetic audit:

```sh
cargo test --test stellar_hold_accuracy audit_synthetic_holds -- --ignored --nocapture
```

The catalog audit reads the TTL stored on each `StarGroup`; synthetic groups use the same configured defaults.
`SPEED_GROUP_TTL_MS` is assigned longest to slowest and shortest to fastest, currently 365, 10 and 1 days.
Constellation stars use the fastest group's TTL. The former error-derived hold formula is no longer used.

**The synthetic audit currently fails qualification intentionally:** it includes an approaching star whose
motion is too fast even for the one-day fast-group hold. The test exits nonzero
after reporting every sampled group if any error exceeds the 0.285-arcsecond target (plus 1e-12 radians of
numerical slack). Ordinary `cargo test` skips the qualification audits and instead runs small regression tests
that check the detector against passing cases and the historical slow-group violation using an explicit
9.5-year hold. With the current defaults, the sampled slow group passes (worst 0.216 arcseconds), while the
fast group and constellation sample reach 11.295 arcseconds. These are synthetic results, not catalog-wide bounds.

Audit every star in the embedded BSC5 catalog, without network access:

```sh
cargo test --test stellar_hold_accuracy audit_embedded_holds -- --ignored --nocapture
```

To examine existing AT-HYG or Gaia data, explicitly supply a prepared `.catalog` file:

```sh
ASTROTERM_HOLD_AUDIT_PREPARED=/path/to/existing.catalog \
  cargo test --test stellar_hold_accuracy audit_local_prepared_holds -- --ignored --nocapture
```

The local audit reads the file using its stored fingerprint and the normal format, checksum and semantic
checks. It does not resolve a dataset, download missing blocks, validate source freshness, rebuild caches or
write files. An incompatible or damaged prepared catalog produces an error. The entire supplied catalog is
loaded and grouped in memory; using a small prepared selection keeps startup and memory costs manageable.
By default, it checks the first 1,000 stars in each speed group across region/table order, plus up to 1,000
constellation stars. `ASTROTERM_HOLD_AUDIT_STARS_PER_GROUP` sets a positive limit for each category. The report
shows actual counts; this prefix sample is not a random or catalog-wide qualification.

For each sampled star, dates cover the computational interval's endpoints, J2000, representative distant
epochs and the vicinity of closest approach. Each anchor starts a fresh cache, then checks forward and backward
time at the anchor, halfway through the hold, at its boundary and at the adjacent representable dates just
inside/outside it. Out-of-interval dates are skipped. The audit includes both reused and freshly refreshed samples.

Passing means **the sampled direction comparisons** met the target. It does not prove every date is safe, and
does not measure brightness error, catalog uncertainty, astronomical-model accuracy, tile rasterization or warp
error. No comparison with an external ephemeris is involved.

Cache expiration uses exact integer millisecond differences; the boundary remains valid and the next millisecond expires. Astronomical TT reference calculations remain floating-point.
