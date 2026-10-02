# Astronomy references

These tools establish independent references and record the current model's errors. They do not certify the
future accuracy targets in `astro::accuracy`. Ordinary Rust tests use checked-in fixtures and need no Python or
network access.

```sh
uv venv /tmp/astroterm-reference
uv pip install --python /tmp/astroterm-reference/bin/python -r scripts/reference/requirements.txt
cargo build --release --example reference_probe
/tmp/astroterm-reference/bin/python scripts/reference/generate.py --probe target/release/examples/reference_probe
```

The default command replays saved Horizons responses. `--fetch` explicitly refreshes the network responses.
`--record-current` explicitly replaces the **current-model regression** constants in
`tests/fixtures/current_positions.rs`; review those changes separately from independent reference changes.
Run `cargo fmt` afterwards. Reference-tool direct dependencies are pinned in `requirements.txt`; the actual
versions, generator hash, parameters, frames, corrections and raw-response hashes are recorded in
`tests/fixtures/reference/references.json`. Each raw response records the kernels Horizons actually used.

`generate.py` reproduces the long-term precession comparison with ERFA, near-date stellar positions, and Boston
Sun/planet/Moon positions. Its long-term synthetic-star recipe uses component routines and Horizons Earth/Sun
states, never `epv00` outside its documented contemporary range. The synthetic input is not a measured real star.
Those component comparisons deliberately omit stellar parallax and light deflection. TT is approximated by TDB
for the vector-query epochs; observer queries explicitly use TT.

**Provider limit discovered during phase 0:** Horizons rejected Earth states at year 12026. Its online service
currently exposes only 9999 BC through AD 9999, although DE441 covers a larger interval. Long-term star/state
fixtures therefore stop at 8026 and the missing 12026 reference is explicitly recorded. ERFA precession fixtures
still include 12026 and 15026. Future validation beyond AD 9999 requires direct DE441 kernels or another documented
independent source; never extrapolate a reference or silently substitute a planetary barycenter for a body center.
See the [Horizons long-term ephemerides documentation](https://ssd.jpl.nasa.gov/horizons/manual.html#long-term-ephemerides).

## Boston audit, 2026-10-03

Observer: geodetic latitude 42.3601°, east longitude −71.0589°, WGS84 height 0. Epoch: UTC JD 2459146.0,
2020-10-23 12:00; Horizons TT JD 2459146.000800741 (TT−UTC = 69.184 s). Airless apparent topocentric coordinates.
The current Rust model still uses TT = UT1 = UTC, lacks nutation/aberration/light-time, uses approximate planetary
elements, and applies an approximate lunar parallax correction. These are current-error measurements, not isolated
attributions to any single missing correction.

| Body | Fresh reference altitude | Current Rust altitude | Angular separation |
|---|---|---|---|
| Sun | 8.342528° | 8.340254° | 9.024″ |
| Mars | −20.409296° | −20.469276° | 221.139″ |
| Neptune | −45.326542° | −45.339796° | 68.253″ |
| Moon | −65.889203° | −65.921966° | 119.110″ |

The old C/Stellarium altitudes for Mars, Neptune and the Moon differ from these fresh references by about 0.82°,
0.66° and 1.78°. The much smaller current-model discrepancies show that the old numbers are unsuitable as precise
accuracy references. The original C tests contain URLs and constants but no saved output or refraction settings;
the historical source of those discrepancies cannot be uniquely reconstructed. The inherited constants and their
tolerances are preserved, annotated, and supplemented by `tests/position_references.rs`.

For Vega, ERFA gives −0.362262° apparent altitude and −0.366119° with corrections matched to the current mean-place
pipeline; Rust gives −0.366117°. The inherited exactly-zero altitude is not an airless reference. It could reflect
display clipping or different atmospheric settings, but the original fixture provides no evidence to choose
between them. Arcturus and Vega agree with independently implemented, matched mean-place calculations within 0.1″.
Their full apparent-place differences are below 30″. The new tests use angular separation rather than subtracting
azimuths, avoiding wrap and pole artifacts.

The precession-only IAU 2006 versus Vondrák rotation differences are 0.0591′ (5026), 0.5043′ (7026), 2.2193′
(9026), 11.9655′ (12026), and 45.8624′ (15026). ERFA frame bias is removed for this comparison. These reproduce
the roadmap's rounded table and do not imply the current model meets the future 0.01″ target.

The phase-0 probe also quantified lunar phase frame mixing. Precessing the Sun into the Moon's nominal
of-date frame changes the elongation by −0.290711° in Boston and −0.351563° at the 2025 fixture, zero at J2000.
This measures the isolated frame correction while retaining the current fixed-obliquity helper. It is not the
final observer-dependent illuminated fraction. Phase 2 now computes that fraction from common-frame vectors.

Sources: [Horizons API parameters](https://ssd-api.jpl.nasa.gov/doc/horizons.html),
[ERFA source/documentation](https://github.com/liberfa/erfa/tree/master/src), and the neighboring C project's
`test/core_test.c` for the inherited fixtures. No online data is consulted by `cargo test`.


Phase-2 lunar illumination uses a separate replayable fixture, preserving the original audit files:

```sh
/tmp/astroterm-reference/bin/python scripts/reference/moon_phase.py --fetch  # explicit network refresh
/tmp/astroterm-reference/bin/python scripts/reference/moon_phase.py          # offline replay
```

`moon_phase_geocentric.request.json` and `.response.json` retain the full Horizons query/response; `moon_phase.json`
records the generator and response hashes. Query dates are TT Julian dates, observer Earth center, quantity 10
(illuminated percent). The geometric model is compared with Horizons apparent illumination at four dates; the
0.001 fraction envelope is a model comparison, not the 1″ cache interpolation budget.
