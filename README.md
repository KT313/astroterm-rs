# astroterm-rs

> [!NOTE]
> This code is ported from [astroterm](https://github.com/da-luce/astroterm) by
> [da-luce](https://github.com/da-luce) (Dalton Luce), and further work on it is inspired by the original project.
> All credit for the original design, algorithms and data preparation goes there.

A Rust port of astroterm (reference: the `feat/facing-view` branch), a terminal star map showing stars, planets, the
Moon and constellations for any date, time and location.

```sh
cargo run --release -- -a 1.29 -o 103.85 -cCu                          # see `--help` for all options
cargo run --release -- -i Tokyo -u -F NNW -T 20 -z 120 -m             # facing view with metadata
source <(cargo run --release -- --bash-completions)                     # bash completions
```

## Renderers

Characters remain the default. Use `--renderer pixels` for a true-color sky with anti-aliased stars, curved
constellations, Sun/planet discs and a continuously shaded Moon lit toward the Sun. Object sizes are schematic,
chosen for visibility rather than angular diameter. Sixel, Kitty and iTerm2 receive one completed bitmap containing
the sky, labels, metadata and warnings. Text is rasterized with bundled DejaVu Sans Mono; it does not use the
terminal's configured font. Text defaults to 85% of the reported cell dimensions; `--text-scale 1` restores the
previous size, while `--text-scale 0.7` makes it smaller or `--text-scale 1.2` makes it larger. The range is 0.25–4;
glyphs, spacing and the metadata layout scale together. Metadata and timing text have transparent backgrounds,
so unused space does not cover the sky. This flag affects Sixel/Kitty/iTerm2 raster text only;
characters and half-block text retain the terminal's font size. Latin/Greek text and common astronomy
symbols are supported; missing glyphs use a replacement character. Complex-script shaping and color emoji are
not implemented. Half-block output keeps native terminal text, merged into the same cell buffer as the sky.

```sh
make build
make run -- --renderer pixels -i Tokyo -d 2025-03-01T11:00:00 -s 0 -C -m
# Test a particular protocol supported by your terminal:
make run -- --renderer pixels --graphics-protocol sixel -i Tokyo -C -m
```

`--graphics-protocol auto|kitty|sixel|iterm2|halfblocks` defaults to automatic selection. Unix terminals are queried
with a bounded timeout; known iTerm2/WezTerm/Rio environments use iTerm2. Other terminals fall back to colored half-blocks.
Kitty output sends RGB images through direct placements. Zlib compression is enabled only after a successful
capability probe, including when Kitty is selected explicitly. With `-m`, the metadata panel reports whether
compression is enabled, unsupported by the terminal, or unconfirmed because the probe was unanswered.
Unsupported or unconfirmed compression falls back to uncompressed RGB. Use a forced protocol to compare your
terminal's implementations; an unsupported forced protocol may show nothing or escape characters.
Graphics startup errors fall back to characters with a visible notice.

Pixel dimensions come from the reported terminal/cell size, with a 10×20-pixel cell fallback. `--aspect-ratio` still
overrides the viewport's cell aspect ratio. Resizing redraws the image; quit and panic restore the terminal and
remove both of this application's Kitty images. Pixel rendering always uses true color; `--color`, `--unicode` and
`--braille` apply only to characters. Constellations, grid, magnitude/label thresholds, dynamic names, refraction,
metadata and accuracy warnings apply to both renderers.

Pixels default to 12 fps to allow for image encoding and transport; characters retain 24 fps.
Use `--fps 24` (or another value) to override the default. `--debug-frametimes` separates rasterization,
text layout/rasterization, pixel conversion, encoding, image composition, frame serialization and presentation.
Kitty presentation additionally separates image upload and image swap. The scene is assembled
in named passes (canvas, horizon, stars, constellations, planets, Moon, grid, labels, metadata and notices).
Full graphics paints all text into the screen-sized bitmap before encoding. Half-blocks merge text cells before
serialization. Kitty uploads the next image while the current image remains displayed, then uses a short
synchronized update (DEC mode 2026) to place the completed image and delete the previous one. Two alternating
image IDs bound terminal storage. Other protocols wrap their completed output in a synchronized update.
Protocol transport/cleanup is covered by PTY checks; actual image appearance depends on the terminal and is
tested separately. See [terminal checks](scripts/checks/README.md).

## Keys

| Key | Action |
|---|---|
| arrows, `h` `j` `k` `l` | Look around (turns the overhead view into the identical facing view first) |
| `+` `-` | Zoom in / out |
| space | Pause / resume time |
| `]` `[` | Speed time up / slow it down (10x per press) |
| `r` | Reverse time |
| `0` | Reset the view |
| `q`, Esc, Ctrl-C | Quit |

With `--quit-on-any`, any key quits instead.

Dates use the **proleptic Gregorian calendar** and astronomical year numbering: year `0` is 1 BC, `-1` is 2 BC.
Use signed extended years with `-d`, for example `-7974-01-01T00:00:00` or `+12026-01-01T00:00:00`. Input is UTC
(UT before UTC existed); currently the calculations approximate UT1 and TT by that same input time. Wide calendar
support is not an accuracy guarantee. Accuracy targets and currently unvalidated ranges are documented in
`astro::accuracy`; long-term astronomy improvements remain planned.

## Layers

Each module only depends on the ones above it:

| Module | Responsibility |
|---|---|
| `timing` | Smoothed frame durations and opt-in single-frame execution traces |
| `astro` | Julian dates, sidereal time, precession, coordinate conversions, star/planet/Moon positions |
| `canvas` | In-memory cell grid (clipping, wide glyphs, braille merging) and line drawing |
| `projection` | Stereographic / equidistant projections onto the unit disk and the `View` (zenith or facing, fov) |
| `catalog` | Embedded BSC5 star catalog, star names, constellation figures, cities, orbital elements |
| `sky` | Object model (`Sky`, `Star`, `Planet`, `Moon`) and the per-frame position update, without rendering details |
| `controls` | Actions the user can trigger (`Control`) and their effect on the view and the simulation clock |
| `metadata` | What the metadata panel shows (date, zodiac, Moon phase, location, time, speed, view), as fields |
| `scene` | Character-grid rendering and pure RGBA rasterization, appearance and orientation aids |
| `terminal` | Character/pixel renderer enum, protocol detection, text overlays, input and session restoration |
| `cli` | Arguments, validated `Config` (simulation, view, render and terminal settings), bash completions |

`src/main.rs` holds the processing flow: parse options → build the sky → per frame: poll input and apply controls,
refresh simulation → prepare observer/emission samples → observe → project → render. The renderer enum selects
characters or pixels; both read the same observed sky. Projection uses cell or pixel viewport units respectively.

Within observation, `src/sky/observation.rs` explicitly sequences region filtering, conservative brightness bounds,
body sampling, candidate validation, constellation endpoint inclusion, stellar motion, current brightness filtering,
observer subtraction, Moon illumination, aberration, horizon rotation and optional refraction. Each pass has a
dedicated function and timer. `src/projection/sky.rs` separately times visible-star projection, draw-order sorting,
body projection, constellation projection and horizon projection. Exact visibility is checked after corrections;
early filtering is conservative, and constellation endpoints remain available even when not drawable as stars.

## Differences from the C version

New:

- Optional pixel rendering through Kitty RGB or ratatui-image (Sixel, iTerm2 or half-blocks), with tiny-skia drawing and
  fontdue text rasterization. Full graphics encodes one completed bitmap; half-blocks use merged text/image cells.
  Kitty uses capability-checked compression and uploads before switching images. Both renderers share the curved
  constellation geometry and lunar lighting direction.
- Stars move along normalized 3D trajectories in Julian years (365.25 days). With known distance, their magnitude
  changes with distance; without it, motion is tangential and brightness stays constant. Approaching-star bounds
  include perspective acceleration.
- Projection consumes horizontal unit vectors directly and produces Cartesian screen coordinates.
- Compact immutable star arrays and a conservative cube-map grid limit observation to possible visible stars;
  fast movers and constellation endpoints are handled independently. Stars rejected by current brightness are
  removed before direction corrections unless needed as constellation endpoints. Per-frame positions stay separate.
- Simulation, observation, camera projection and rendering are separate stages. Planetary, lunar and orientation
  models have independent caches; Earth rotation and observer corrections follow simulation time, so panning while
  paused does not trigger an ephemeris update. Configurable processing caches retain separate stage results;
  `--disable-cache` provides a per-frame direct-evaluation reference.
- Moon illumination uses Sun/Moon vectors relative to the observer in one frame, with a continuous illuminated
  fraction and phase angle. Full Moon is recognized on both sides of opposition within the phase band.
- Interactive controls (see [Keys](#keys)); the metadata panel shows the simulation speed and whether time is paused.
- With `--color`, stars are colored by spectral class (blue-white O/B in cyan, orange K in yellow, red M in red).
- VSOP87E supplies barycentric states for the Sun and planets, including the Earth geocenter. The Moon uses the
  120-term Meeus chapter 47 series (truncated ELP-2000/82), adapted from its native mean ecliptic of date.
- Exact topocentric geometry subtracts a WGS84 sea-level site for every finite body. Observation applies iterated
  light-time and annual/diurnal aberration. Lunar glyphs face the Sun in the current view.
- TT = UT1 + Espenak–Meeus ΔT drives the models; UTC input approximates UT1. Vondrák long-term precession,
  IAU 2000B nutation and a matching equation of origins keep the equinox consistent with Earth rotation.
- A yellow bottom-row message appears whenever any drawn class is outside its measured accuracy range;
  it stays stable while panning. See [Accuracy](#accuracy) for ranges and limitations.
- Star and planet positions are precessed from J2000 to the date, so they line up with the sidereal time of date
  (the C version was about 0.35° off in 2025, growing by about 1.4° per century away from 2000).
- Optional atmospheric refraction (`-R`/`--refraction`), which lifts objects near the horizon by up to about 0.647°.
- `--dataset athyg` downloads the pinned AT-HYG catalog on first use and reuses it offline thereafter.
  `--dataset <path>` loads stars from an AT-HYG file (`.csv` or `.csv.gz`, see Data Sources) instead of the embedded
  Yale Bright Star Catalog; constellation figures are matched by HR number. Unnamed stars are labelled with their
  Bayer, Flamsteed, HR, HIP, Tycho-2 or Gaia designation, and stars without a spectral type are colored by B-V.
  For duplicate HR numbers, the brightest original catalog entry (earliest source row on a tie) represents the HR
  in constellation figures and receives its BSC5 magnitude; other components keep their catalog magnitudes.
  Stars that could reach the threshold within the computational interval are candidates; current brightness
  determines drawing and labels. Constellation endpoints are updated independently. Prepared catalogs are cached
  in the per-user cache folder; valid caches are memory-mapped at startup.
- Dynamic names: when fewer than 5 objects in view have labels (e.g. after zooming in), the brightest stars in view
  are named too, with their catalog number (`HR 1234`) if they have no proper name. `--disable-dynamic-names` turns
  this off.
- `--debug-frametimes` shows Simulation, Observation, Projection, Draw and Present with their timed sub-steps as
  exponential moving averages below the metadata. Observer geometry and light-time sampling have separate timers.
  Repeated calls in the same scope are summed per frame before smoothing; identical names under different parents
  stay separate. Parent totals include their children and are counted only once in the frame total. The complete
  breakdown needs a tall terminal.
- `--debug-singleframe` presents one frame, restores the terminal, and prints an ordered pipeline report with
  unsmoothed timings, input/output counts, filtering reasons, cache decisions and image transport sizes.

Fixed:

- Star labels are chosen at draw time instead of being erased from the star table while rendering.
- Braille constellation lines merge within canvas cells; no global 1024x1024 buffer.
- Constellation lines are clipped exactly as great-circle arcs against the view, so only the parts actually in view
  are drawn. Adaptive sampling follows their projected curvature in both renderers, instead of straight chords.
- Simulation time follows the wall clock (no drift when frames are slow).
- Datetimes are parsed as UTC without `mktime`, so local DST no longer shifts them.
- Gregorian date conversion also works before 4800 BC; extended years keep their sign in the metadata panel.
- Non-finite CLI numbers are rejected; interactive speed changes saturate at ±10¹² to avoid overflow.
- Mean anomalies wrap correctly for large negative values.
- The point directly behind an equidistant view has a fixed direction instead of a random one.
- The 14 BSC5 placeholder entries (no data) are no longer drawn as a bright star at RA 0 / Dec 0.
- Grid spokes are sorted with a valid comparator; Ctrl-C quits cleanly; the terminal is restored on panic.
- Metadata uses the **observer's** geographic time zone, found offline with `tzf-rs`, and the abbreviation in effect
  at the simulated date (system IANA rules via `tz-rs` on Unix). Ocean polygons may assign nautical `Etc/GMT` zones.
  Missing zones or unavailable rules (including non-Unix platforms) show UTC with "(no timezone found)".
  Negative coordinates between 0° and -1° keep their sign (`-0° 30' 0.00").
- City lookup also ignores the case of non-ASCII letters and suggests up to three similar names for misspellings.

AT-HYG numeric inputs must be finite, with RA in [0, 24) hours and declination in [−90, 90] degrees; malformed
values report the source line. Rows missing RA, declination or magnitude are skipped. Incomplete Cartesian triples
are treated as missing. Distance must be positive, below 100,000 pc and, when a full position triple exists, agree
with its length within 1%; otherwise the star uses angular proper motion only. Valid distance with missing position
uses RA/Dec; missing velocity uses tangential proper motion plus radial velocity (zero if absent). These validated
3D inputs drive straight-line space motion and distance-dependent brightness. Initial direction uses precise
RA/Dec, since some Cartesian positions in the file are rounded. Tangential RA motion is retained at the poles.
Names are catalog-owned, and stable IDs preserve source identity through sorting.

Trajectory preprocessing uses the shared computational interval. A closest approach below 0.001 of the initial
distance drops the radial component and distance information for that star, keeping brightness constant. A load-time
count is printed to stderr; `--debug-frametimes` also shows this count and any per-frame fallbacks outside the interval.
Outside the interval every star is checked; expired brightness keys never suppress a newly bright star.

Selection uses each trajectory's brightest possible magnitude; drawing sorts only visible stars by current
brightness, with stable-ID ties. Dynamic labels use the reverse order. Constellation endpoints join the update set
before refraction, so all objects receive the correction exactly once. Stars with motion bounds above 15′ enter
an always-checked list. Other stars are grouped into depth-6 cube-map cells, queried at depth 4 for wide views
and depth 6 for narrow ones; fields of view of at least 300° use all cells. The query includes conservative motion,
quantization, refraction and reserved aberration margins. Exact current brightness and projection decide visibility.
Draw-order comparisons use compact current-magnitude/ID/index records. The projection cache reuses sorting
scratch capacity; `--disable-cache` still rebuilds the order each frame.

Directions and scaled velocities are stored as `f32` and expanded for `f64` evaluation. Brightness keys round
brighter and angular bounds round outward. Trajectories whose full-interval quantization bound exceeds 0.5″ stay in
a sparse `f64` exception table; near-collision handling is decided from the effective stored trajectory. Names,
designations and IDs are separate from the numerical arrays. `--debug-frametimes` reports candidate cells and
stars, evaluated stars, and stage times; substeps are included in their parent stage only once in the frame total.

## Datasets and cache

The embedded BSC5 catalog remains the default and needs no download. To use AT-HYG:

```sh
astroterm --dataset athyg -i Tokyo -cCu
astroterm --dataset ./datasets/athyg_40.csv.gz -i Tokyo -cCu
```

`athyg` selects AT-HYG v4.0: 199,688,001 compressed bytes (about 200 MB), verified with SHA-256
`69ad04dd33d7c7bb4f5e1b4682798075811547ea9fb8d0e802e5b319c46818a6` before installation. Progress is printed before
the terminal opens. An existing file takes precedence over a dataset name; values containing `/` or `\` are paths.
Unknown bare names are errors. Use `./filename` for a missing relative file. Existing named downloads work offline;
failed downloads report the source URL, destination and manual `--dataset <path>` alternative.

Downloads and prepared caches use the operating system's per-user data and cache folders, respectively. On Linux:

- data: `$XDG_DATA_HOME/astroterm/athyg_40.csv.gz`, default `~/.local/share/astroterm/athyg_40.csv.gz`;
- cache: `$XDG_CACHE_HOME/astroterm/`, default `~/.cache/astroterm/`.

No cache is written beside your dataset. Source path, size and modification time identify a cache; preprocessing
rules and supplemental catalog data have their own fingerprint. The cache has a checksum and structural/semantic
checks. A stale or damaged cache is rebuilt from the CSV. Cache-writing failures are reported but do not prevent
use of the dataset. Downloads and cache files are installed atomically; failed operations remove their temporary
files. The cache can be deleted safely while the application is closed. Only immutable catalog data is mapped;
observed positions and projected frames remain ordinary mutable buffers. The current cache format is supported on
64-bit little-endian systems; other targets use the CSV path without caching. Named datasets are not automatically
updated to a different upstream version.

## Processing caches

Runtime processing caches are separate from the downloaded dataset and prepared catalog files above. Policies
are loaded once from the platform configuration directory: on Linux, `$XDG_CONFIG_HOME/astroterm/cache.toml`,
usually `~/.config/astroterm/cache.toml`. A missing default file uses built-in defaults. Use
`--cache-config <path>` to select an explicit file; missing explicit files and invalid policies fail at startup.
See [examples/cache.toml](examples/cache.toml) for all supported groups.

| Group | Maximum offset from sample epoch |
| --- | --- |
| Intrinsic stellar state | 360 simulated seconds, shortened to respect a 0.1″ direction allowance |
| Planetary samples, including Sun and Earth | ±30 simulated seconds |
| Lunar samples | ±12 simulated seconds |
| Slow orientation | ±60 simulated seconds |

Durations use TT, in either playback direction. Hits never extend a sample's validity. Shorter intervals and
individual group disabling are supported; larger unqualified intervals, non-finite/negative values, unknown keys
and TTLs on dependency-only groups are rejected. Planetary/lunar states are evaluated at the requested epoch from
position/velocity samples; their displayed positions are not frozen until the next model refresh.

The initial stellar brightness guard is deliberately conservative: moving stars with known distance are evaluated
at each distinct epoch, preserving current magnitudes, threshold crossings and draw order. Constant-brightness
trajectories can reuse their intrinsic state within a checked angular bound. This means the six-minute setting
is a maximum, not a guarantee that every star will be held that long. Region selection includes the extra angular
allowance. Entries are filled lazily and bounded by catalog size; physical state is retained when the camera moves.

Each observation correction owns a separate result. Projection caches own geometry and bind star references only
for the current frame. Pan/zoom, site changes, resizing and changed upstream inputs invalidate the relevant results.
Both renderers can reuse an unchanged sky canvas; labels/metadata and graphics presentation remain correctly
assembled. Continuous Earth rotation normally requires new projection and rasterization during playback.

`--disable-cache` bypasses **runtime processing reuse**, including existing model interpolation and retained glyph
masks. Active stages execute every frame, even while paused. Identical requests may share frame-local results;
allocated buffers, loaded fonts and immutable catalog inputs remain reusable. It does not delete cache files,
disable the startup catalog mapping, or redownload the dataset. `--debug-frametimes` includes lifetime
`H` (hit), `R` (run) and `B` (bypass) counts, with stage timings still including the cache checks. The no-cache mode uses the same models;
it is not a claim of exact physical astronomy.

## Development

Use `--debug-singleframe` to inspect one real frame, including startup, observation, projection, rendering,
text and presentation:

```sh
make run -- -i Tokyo -d 2025-03-01T11:00:00 -t 5 -C --debug-singleframe
make run -- -i Tokyo -d 2025-03-01T11:00:00 -t 5 -C --renderer pixels --debug-singleframe
```

Before entering the frame loop, `main.rs::prepare_frame_data` prepares catalog-only inputs once: stellar motion
classifications, reusable constellation endpoint topology, and per-star RGB/character colors plus name eligibility.
The `Frame preparation` trace records these startup costs separately from frame timings. The additional fixed
per-star tables use approximately six bytes per catalog star on a 64-bit build (about 15.4 MB for AT-HYG).
The prepared data belongs to its catalog: replacement catalogs or changed constellation definitions use correct
fallback calculations. Library callers can opt in through the cache/renderer `prepare_catalog` methods.

Pixel raster-key construction collects named-label candidates using the prepared flags; text layout visits that
small list plus dynamically selected labels, retaining the original drawing order. Current magnitude, visibility,
label choice, drawing order, projected arcs, and brightness-dependent pixel sizes/intensities remain dynamic.
Designation strings are still formatted only for selected labels; startup does not expand millions of unused names.
Immutable preparation remains available with `--disable-cache`, like the prepared catalog itself.

The report goes to stdout after the terminal is restored. This flag does not enable the metadata panel;
`-m` and `--debug-frametimes` still work independently. It uses the exact requested start epoch even with a
nonzero simulation speed, presents once, and exits without the frame-rate sleep. Normal terminal negotiation
still runs. Add `--disable-cache` to bypass runtime reuse; it does not bypass the prepared catalog cache.

Entries follow invocation order, with indentation for nested stages; repeated light-time sampling calls stay
separate. Filters report their ordered rejection counts, and endpoint-only stars are distinguished from drawable
stars. Geometry counts describe submitted objects, not unique visible pixels after clipping and overdraw.
These are first-frame wall times, including cold runtime caches. Parent timings include their children and
extra diagnostic bookkeeping, so do not add all rows or compare directly with steady-state frame averages.
Extra diagnostic scans and formatting are disabled during ordinary animation. Each parent reports direct-child
costs, directly enclosed diagnostics, and the remaining self/unattributed time. Diagnostic work outside any
stage is reported separately. A remainder is visible overhead or uncovered work, not automatically calculation time.

Stellar work runs in batches of at most 1,024 stars: cache lookup/decisions, trajectory reads, motion and magnitude
calculation, validity qualification, cache stores, and output assembly. Child timings are sums across batches in
first-occurrence order; these passes interleave for each batch. There are no per-star clocks or per-star trace rows.
The report counts refreshed/reused stars, zero-validity reasons, and extra model evaluations for validity checks.
This decomposition uses bounded scratch storage and an additional cache lookup for each refreshed star's store;
it preserves numerical results and cache policy but changes traversal costs, so old fused-loop timings are not
an identical implementation baseline.

Raster cache-key construction, cache decisions/stores, image copies, projected-view assembly, sorting,
endpoint-index merging, calculated-state construction, and direction capture/restoration have separate timers.
Working selections contain only catalog indices and drawing eligibility. Observed star buffers contain calculated
state; renderers borrow immutable metadata directly from the shared catalog. Refresh calculations retain their
output, while direction restoration runs only on cache hits.

Raster keys use exact ordered drawing inputs instead of full observed records: pixel keys contain cell, current
magnitude and base RGB; character keys contain cell, glyph and color plus the resolved text of selected labels.
Names remain outside the pixel sky cache because text is composed separately. Keys still require a linear scan
and exact comparison; this is a memory/copy reduction, not constant-time invalidation. On a 64-bit build, pixel
star keys use 24 bytes instead of 120, working selections use 16 instead of 104, and calculated star records use
48 instead of 104. Borrowed projected-star records increase from 24 to 32 bytes because they also reference the
catalog. These are element sizes, not total process memory or measured speedups.

CSV loads report skipped rows by reason; embedded BSC loads report placeholder removal. A mapped catalog
reports its validated star count and explicitly marks original CSV skip counts unavailable: that cache format
does not store them, and tracing does not reread the source to reconstruct them. Invalid source values remain
loading errors, not silently skipped rows. Library reference paths need not emit all application diagnostics;
the report covers the named stages of the production frame pipeline, not every scalar math helper.

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```

Offline scene snapshots include colors and wide-glyph occupancy. Independent astronomy fixtures and the Boston
reference audit are described in [scripts/reference/README.md](scripts/reference/README.md). Reproducible
benchmarks and PTY checks are described in [scripts/checks/README.md](scripts/checks/README.md).

The four-stage implementation lives in `sky/simulation.rs`, `sky/observation.rs`, `projection/sky.rs`, and `scene/`.
Pure formulas and coefficients live in `astro/models/{stars,planets,moons,orientation}`. Body identity is independent
of the formula used. Star inputs are shared across observers; projected output borrows the immutable observed sky.
The legacy `update_sky_positions` API remains a direct, uncached reference convenience; the application uses the
explicit stages in `main.rs`, with common runtime policy/validity code in `cache/` and typed cache owners in the
observation, projection and scene modules. Observation accepts a renderer-neutral `SkyRegion`; use `All` when the same observed
sky must support arbitrary subsequent camera views.

Current cache half-intervals are 30 simulated seconds for the planetary batch, 12 seconds for the Moon, and 60 seconds
for slow Earth orientation, forwards or backwards. Observation evaluates all samples at one requested epoch;
parent-relative lunar states are composed with Earth at that same epoch. Bounded additional samples cover
per-body emission epochs. The observer stays at reception while each target moves to emission; an initial
light-time estimate plus one iteration requests bounded samples explicitly. Observation then reads those samples
without invoking ephemeris code. Missing coverage is an error returned to the coordinator. Outside the computational interval, caches use direct evaluation only.

Observation borrows prepared catalog arrays once per pass. `ObservedStar` contains only `source_index`,
`drawable`, current `magnitude` and `position`. Library callers access static metadata through
`sky.star_view(index)` or `sky.star_views()`: `id()`, `name()`, `designation()`, `spectral_type()` and `color_index()`.
`ProjectedStar.star` is an `ObservedStarView` that borrows both calculated state and catalog metadata.
`designation()` returns an `EncodedDesignation`; call `.resolve()` when a label needs its enum value. Catalog
input records retain `Option<Designation>`. Change static metadata in the input `Catalog` before preparing a sky;
filtered/reordered calculated states must retain valid source indices into their owning catalog.
The published observed sky contains drawable candidates and required constellation endpoints; `CorrectionStats`
records the number prepared, skipped and retained only as endpoints. `--debug-frametimes` exposes the separate
Correction selection stage and these counts. Stellar aberration accepts the already normalized intrinsic direction;
distance-vector corrections retain their generic normalization path.

The cache interpolation budget is 1″ (0.3″ planetary direction, 0.4″ lunar direction, 0.2″ orientation, 0.1″ velocity
expressed as aberration). This is measured **against the current models**, not an astronomical accuracy claim.
Positions and velocities are barycentric, equatorial J2000, in AU and AU/day. Each family owns its formulas and
accuracy policy. The Moon is sampled relative to Earth, then composed with its parent at the requested epoch.
The orientation family owns the WGS84 shape and slow precession/nutation; observation transforms the site's
position and rotation velocity with the complete orientation every frame. All current physical calculations use
f64. Archived Kepler/Schlyter APIs exist only for historical reference comparisons.

`make build-aggressive` builds `target/aggressive/astroterm` using fat LTO, one codegen unit, `panic = "abort"`,
and `-C target-cpu=native`. It requires no PGO/BOLT tools or training runs.

`make build-aggressive-pgo` adds profile-guided optimization and BOLT, trained using typical workloads in a
pseudo-terminal (`scripts/pgo-training.sh`), and writes `target/aggressive-pgo/astroterm`. It needs
`cargo install cargo-pgo`, `rustup component add llvm-tools-preview` and BOLT (on Ubuntu `sudo apt install bolt-18`).
Both targets preserve the application's rendering behavior. Select the desired binary explicitly, for example:

```sh
make run BINARY=target/aggressive/astroterm -- -i Tokyo -cCu
```

## Accuracy

The computational interval is −7974-01-01 through 12026-12-31 (Gregorian TT, astronomical years). It bounds
catalog/index shortcuts, **not astronomical accuracy**. Outside it the program checks every star directly.
Empirically qualified ranges are narrower: stars use that whole interval, Sun/planets use **1850-01-01 to
2030-01-01**, and the Moon **0000-01-01 to 4000-01-01** (upper endpoints exclusive). A single warning appears
outside any of these ranges. The bands below remain targets, not promises throughout the full band.

Independent qualification uses 659 sampled epochs, past and future separately, with observation 59 seconds after
a cache sample. DE441 vectors come directly from JPL kernel byte ranges, including year 12026; ERFA provides
orientation/aberration references. A separate 27-position Horizons check uses actual planet centers, WGS84 Boston,
matched TT and UT1, and no refraction. It measures planets below 1.5″ and the Moon below 4.9″. The broad DE441
check uses explicitly identified system barycenters for Mars and the outer planets. Results below are sampled
maxima, not rigorous bounds between samples or guarantees for every observer/catalog trajectory.

| Band from J2000 | Stars past / future | Sun/planets past / future | Moon past / future |
|---|---:|---:|---:|
| Within 200 Julian years | 0.03″ / 0.03″ | 2.11″ / 3.89″ | 11.45″ / 10.88″ |
| 200–2,000 years | 0.03″ / 0.03″ | 44.61″ / 21.99″ | 59.78″ / 6.49″ |
| Beyond 2,000 years | 0.03″ / 0.03″ | 9,716″ / 17,870″ | 4,526″ / 2,764″ (unvalidated) |

Targets are 1″/1″/5″ for stars, 2″/60″/80″ for Sun/planets, and 15″/120″/unvalidated for the Moon.
Precession agrees with ERFA's long-term matrix to below 0.000001″ at the sampled epochs; this is implementation
agreement, not a claim that Earth's distant orientation is known that accurately. Neptune misses the near-band
2″ target at some dates, and long-term outer-planet errors exceed 80″. Hence the conservative planetary range.
Meeus meets the sampled lunar targets inside its qualified range; its far-date comparisons remain unvalidated.

Time-scale uncertainty is separate: UTC is approximated as UT1 (up to 0.9 s, about 13.5″ of rotation), and ΔT is
an estimate, especially before modern measurements or in the future. The polynomials give about 75.1 s in 2026,
4.3 hours in 4026, and 3.9 days in 12026; these are not measured future rotation. Model comparisons explicitly
match TT and UT1 to avoid conflating these uncertainties. TT≈TDB neglects the small periodic difference.
Catalog uncertainty, stellar parallax, gravitational light deflection, observer elevation and polar motion are
not modelled. Refraction is an optional standard formula, not a weather measurement.

Reproduction commands, reference provenance and the interpretation of the ranges are in
[scripts/reference/README.md](scripts/reference/README.md). Ordinary tests are offline.

## Citations

Resources used by the original astroterm and this port:

- [astroterm](https://github.com/da-luce/astroterm) by Dalton Luce, the project this code is ported from
- [Map Projections - A Working Manual by John P. Snyder](https://pubs.usgs.gov/pp/1395/report.pdf)
- [Wikipedia](https://en.wikipedia.org)
- [Atractor](https://www.atractor.pt/index-_en.html)
- [Jon Voisey's Blog: Following Kepler](https://jonvoisey.net/blog/)
- [Celestial Programming: Greg Miller's Astronomy Programming Page](https://astrogreg.com/convert_ra_dec_to_alt_az.html)
- [Practical Astronomy with your Calculator by Peter Duffett-Smith](https://www.amazon.com/Practical-Astronomy-Calculator-Peter-Duffett-Smith/dp/0521356997)
- Astronomical Algorithms by Jean Meeus
- [NASA Jet Propulsion Laboratory](https://ssd.jpl.nasa.gov/planets/approx_pos.html)
- [Paul Schlyter's "How to compute planetary positions"](https://stjarnhimlen.se/comp/ppcomp.html)
- [Dan Smith's "Meeus Solar Position Calculations"](https://observablehq.com/@danleesmith/meeus-solar-position-calculations)
- [Bryan Weber's "Orbital Mechanics Notes"](https://github.com/bryanwweber/orbital-mechanics-notes)
- [ASCOM](https://ascom-standards.org/Help/Developer/html/72A95B28-BBE2-4C7D-BC03-2D6AB324B6F7.htm)

Additional accuracy-model sources:

- [Bretagnon & Francou (1988), VSOP87](https://ui.adsabs.harvard.edu/abs/1988A%26A...202..309B/abstract),
  through the [VSOP87E Rust implementation](https://docs.rs/vsop87/3.0.0/vsop87/vsop87e/).
- [Vondrák, Capitaine & Wallace (2011/2012), long-term precession](https://www.aanda.org/articles/aa/abs/2011/10/aa17274-11/aa17274-11.html).
- [ERFA](https://github.com/liberfa/erfa): long-term pole and IAU 2000B nutation coefficients; translated code is
  covered by [LICENSE-ERFA](LICENSE-ERFA), with its SOFA heritage acknowledged there.
- [Espenak–Meeus ΔT polynomials](https://eclipse.gsfc.nasa.gov/SEcat5/deltatpoly.html).
- Jean Meeus, *Astronomical Algorithms*, second edition, chapters 22 and 47 (lunar tables 47.A/B).
- [WGS84](https://earth-info.nga.mil/index.php?dir=wgs84&action=wgs84): equatorial radius 6378137 m,
  inverse flattening 298.257223563, height 0.
- [JPL DE440/DE441](https://ssd.jpl.nasa.gov/planets/eph_export.html) and
  [Horizons](https://ssd.jpl.nasa.gov/horizons/manual.html) for independent comparisons.

## Data Sources

The files in `data/` are taken from the original astroterm repository:

- Stars: [Yale Bright Star Catalog](http://tdc-www.harvard.edu/catalogs/bsc5.html)
- Star names: [IAU Star Names](https://www.iau.org/public/themes/naming_stars/)
- Constellation figures: [Stellarium](https://github.com/Stellarium/stellarium/blob/3c8d3c448f82848e9d8c1af307ec4cad20f2a9c0/skycultures/modern/constellationship.fab#L6)
  (converted from [Hipparcos](https://heasarc.gsfc.nasa.gov/w3browse/all/hipparcos.html) to
  [BSC5](http://tdc-www.harvard.edu/catalogs/bsc5.html) indices using the
  [HYG Database](https://www.astronexus.com/projects/hyg), see astroterm's
  [convert_constellations.py](https://github.com/da-luce/astroterm/blob/main/scripts/convert_constellations.py))
- Cities: [GeoNames](https://download.geonames.org/) (filtered and condensed using astroterm's
  [filter_cities.py](https://github.com/da-luce/astroterm/blob/main/scripts/filter_cities.py))
- Time-zone boundaries: [timezone-boundary-builder](https://github.com/evansiroky/timezone-boundary-builder),
  derived from © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright), distributed through
  [tzf-dist](https://github.com/ringsaturn/tzf-dist) under the [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/).
- Planet orbital elements: [NASA Jet Propulsion Laboratory](https://ssd.jpl.nasa.gov/planets/approx_pos.html)

Optional, not distributed with this repository:

- Larger star dataset for `--dataset`: [AT-HYG](https://codeberg.org/astronexus/athyg) (Augmented Tycho-HYG) by
  David Nash / astronexus, about 2.5 million stars from Tycho-2, Gaia DR3 and HYG, licensed
  [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). `--dataset athyg` downloads the pinned v4.0 file
  from its [Git LFS media URL](https://codeberg.org/astronexus/athyg/media/branch/main/data/athyg_40.csv.gz) into the
  per-user data folder. Manual copies can be placed anywhere, including the gitignored `datasets/` folder.

## License

MIT, see [LICENSE](./LICENSE). The original copyright notice of astroterm is kept there.

The unmodified bundled [DejaVu Sans Mono](https://dejavu-fonts.github.io/) font is distributed under its
[Bitstream Vera/DejaVu license](data/fonts/LICENSE-DejaVu.txt); its license notice is also embedded in the font file.
