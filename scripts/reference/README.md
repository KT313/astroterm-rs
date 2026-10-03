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


## Phase 6 qualification

The historical phase-0 audit above is preserved. Production now uses VSOP87E, Meeus chapter 47, WGS84 sea-level
site subtraction, Espenak–Meeus ΔT, long-term precession, IAU 2000B nutation, light-time and aberration.

```sh
cargo build --release --example accuracy_probe --example reference_probe
# Fetch only the required DE441 ranges, about 6 MB for the current audit; never the full 3 GB kernel.
/tmp/astroterm-phase0-venv/bin/python scripts/reference/accuracy.py --full --fetch
# Replay those ranges offline:
/tmp/astroterm-phase0-venv/bin/python scripts/reference/accuracy.py --full
/tmp/astroterm-phase0-venv/bin/python scripts/reference/topocentric.py       # saved Horizons responses
/tmp/astroterm-phase0-venv/bin/python scripts/reference/orientation.py       # ERFA only
/tmp/astroterm-phase0-venv/bin/python scripts/reference/record_current.py    # explicitly replace phase-6 baseline
cargo test --release --test accuracy_models
```

Use any environment installed from `requirements.txt`; the `/tmp` path is an example. `accuracy.py --ranges PATH`
chooses the raw-range directory (default `dev/phase6-de441-ranges`, ignored). Each byte range is saved with its
URL, offsets, HTTP ETag/Last-Modified and SHA256. Fetching refuses servers that ignore the HTTP Range header.
`accuracy.json` contains numerical references and the range provenance manifest; tests need neither Python nor
kernel files. Replaying verifies hashes before evaluating coefficients. `de441.py` uses jplephem only to parse the
DAF directory and NumPy Chebyshev evaluation, independently of the Rust models. The original full kernel is not
redistributed. A rerun of `--fetch` reuses existing verified ranges; remove the chosen range folder explicitly to
request a fresh upstream version.

The 659 reference epochs include the roadmap's eight years, exact computational/range boundaries, seasonal
samples through 1800–2200, fortnightly contemporary lunar cycles, monthly samples every five years in
1850–2030, and past/future middle/far samples. Frame comparisons occur 59 seconds after the seeded caches.
The broad reference assembles DE441 retarded positions, WGS84 geometry, ERFA `ab`, `ltpb`, `numat`/`nut00b`, and
ERA. No light deflection or refraction is included. Its slow origin follows ERFA's long-term equator pole with
64-node Gaussian integration; Rust uses independent composite Simpson integration and its translated pole code.
Contemporary sidereal time/full rotation also have separate `gst06a`/`c2i06a` fixtures.

The broad tests deliberately set explicit TT=UT1 to isolate model error: this is a controlled comparison, not
an application time-scale conversion. SPK epochs use TDB≈TT; contemporary observer checks use Horizons TT and
its saved TDB−UTC and DUT1 values. **Quantity 30 is TDB−UTC after 1962, not TDB−UT1**; quantity 49 supplies DUT1.
The residual TT−TDB difference contributes at most about 0.03″ of rotation near today. Horizons also includes
measured Earth-orientation and light-deflection corrections absent from this application's approximation.

Mars and outer-planet DE441 entries are explicitly system barycenters, not silently substituted center kernels.
The separate 27-position Horizons fixture checks all drawn body centers near today (including Venus and Moon).
Heliocentric vectors are compared by subtracting each theory's Sun: absolute barycentric origins in VSOP87's
older mass/ephemeris fit and DE441 differ at the ~1,000 km level, mostly canceling in observer-relative geometry.

Measured maxima and conservative empirical coverage are in README Accuracy. The Moon meets the near/middle
sampled targets, so the optional ELP-MPP02 replacement was not triggered. Neptune fails 2″ at parts of the near
band; outer-planet VSOP87 extrapolation fails 80″ far from today. The far Moon is recorded, never certified.
Coverage is deliberately 1850–2030 for planets, 0–4000 for Moon, and the computational interval for the stellar
transformation. These are sampled model checks, not guarantees at every untested instant. Reference catalog
uncertainties and unknown future ΔT remain separate. Raw summary output lives in ignored development notes.

Original C numerical fixtures/tolerances remain unchanged. `current_positions.rs` retains the phase-0 historical
baseline; `phase6_positions.rs` is the explicitly regenerated production regression baseline. Four character
snapshots change by a few cells as corrected star/endpoint positions cross rounding boundaries; the remaining
scene snapshots are unchanged. A separate lossless snapshot covers the yellow coverage message and boundaries.
