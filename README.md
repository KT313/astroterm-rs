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
| `timing` | Smoothed durations of the steps of each frame (`--debug-frametimes`) |
| `astro` | Julian dates, sidereal time, precession, coordinate conversions, star/planet/Moon positions |
| `canvas` | In-memory cell grid (clipping, wide glyphs, braille merging) and line drawing |
| `projection` | Stereographic / equidistant projections onto the unit disk and the `View` (zenith or facing, fov) |
| `catalog` | Embedded BSC5 star catalog, star names, constellation figures, cities, orbital elements |
| `sky` | Object model (`Sky`, `Star`, `Planet`, `Moon`) and the per-frame position update, without rendering details |
| `controls` | Actions the user can trigger (`Control`) and their effect on the view and the simulation clock |
| `metadata` | What the metadata panel shows (date, zodiac, Moon phase, location, time, speed, view), as fields |
| `scene` | Character-grid rendering: glyphs and colors, drawing the sky and orientation aids, the panel layout |
| `terminal` | `TerminalRenderer`, key bindings, input, raw-mode session guard, diffing presenter (crossterm) |
| `cli` | Arguments, validated `Config` (simulation, view, render and terminal settings), bash completions |

`src/main.rs` holds the processing flow: parse options → build the sky → per frame: poll input and apply controls,
update positions, render. Rendering only reads the sky, so other renderers can be added beside `TerminalRenderer`.

## Differences from the C version

New:

- Stars move along normalized 3D trajectories in Julian years (365.25 days). With known distance, their magnitude
  changes with distance; without it, motion is tangential and brightness stays constant. Approaching-star bounds
  include perspective acceleration.
- Projection consumes horizontal unit vectors directly and produces Cartesian screen coordinates.
- Compact immutable star arrays and a conservative cube-map grid limit observation to possible visible stars;
  fast movers and constellation endpoints are handled independently. Per-frame positions stay separate.
- Simulation, observation, camera projection and rendering are separate stages. Planetary, lunar and orientation
  models have independent caches; Earth rotation and observer corrections run every frame, so panning while paused
  does not trigger an ephemeris update.
- Moon illumination uses Sun/Moon vectors relative to the observer in one frame, with a continuous illuminated
  fraction and phase angle. Full Moon is recognized on both sides of opposition within the phase band.
- Interactive controls (see [Keys](#keys)); the metadata panel shows the simulation speed and whether time is paused.
- With `--color`, stars are colored by spectral class (blue-white O/B in cyan, orange K in yellow, red M in red).
- The Moon includes the main perturbations by the Sun (about 0.1° instead of several degrees off), parallax, a
  phase computed from its actual elongation from the Sun, and an emoji lit on the side facing the Sun in the current
  view (the C version only mirrored it by hemisphere).
- Star and planet positions are precessed from J2000 to the date, so they line up with the sidereal time of date
  (the C version was about 0.35° off in 2025, growing by about 1.4° per century away from 2000).
- Optional atmospheric refraction (`-R`/`--refraction`), which lifts objects near the horizon by up to about 0.5°.
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
- `--debug-frametimes` shows how long each step of a frame takes (position calculation, drawing, writing to the
  terminal), including Simulation, Observation and Projection with per-family sub-steps, as exponential moving averages below the metadata, to find what needs optimizing.

Fixed:

- Star labels are chosen at draw time instead of being erased from the star table while rendering.
- Braille constellation lines merge within canvas cells; no global 1024x1024 buffer.
- Constellation lines are clipped exactly as great-circle arcs against the view, so only the parts actually in view
  are drawn. Straight lines between projected stars could cut across the whole display in views wider than 180°.
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

## Development

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
explicit stages in `main.rs`. Observation accepts a renderer-neutral `SkyRegion`; use `All` when the same observed
sky must support arbitrary subsequent camera views.

Current cache half-intervals are 5 simulated minutes for the planetary batch, 2 minutes for the Moon, and 6 hours
for slow Earth orientation, forwards or backwards. Observation evaluates all samples at one requested epoch;
parent-relative lunar states are composed with Earth at that same epoch. Bounded additional samples cover
explicit per-body emission-time queries, although light-time correction itself remains unimplemented. Missing coverage is an
error returned to the coordinator. Outside the computational interval, caches use direct evaluation only.

The cache interpolation budget is 1″ (0.3″ planetary direction, 0.4″ lunar direction, 0.2″ orientation, 0.1″ velocity
expressed as aberration). This is measured **against the current models**, not an astronomical accuracy claim.
The underlying Kepler/Schlyter models, zero ΔT and altitude-only lunar parallax remain approximations. The Sun is
still the heliocentric origin; observer site displacement remains zero until the exact-site model is added.

`make build-aggressive` builds a faster binary for the current machine into `target/aggressive-pgo/astroterm`:
fat LTO, one codegen unit, `panic = "abort"`, `-C target-cpu=native`, then profile-guided optimization and BOLT,
trained by running typical workloads in a pseudo-terminal (`scripts/pgo-training.sh`). Behavior is the same as the
release build. It needs `cargo install cargo-pgo`, `rustup component add llvm-tools-preview` and BOLT (on Ubuntu
`sudo apt install bolt-18`).

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
