# Terminal and performance checks

Pixel checks and the phase-7 prototype:

```sh
cargo build --release --bin astroterm --example terminal_probe --example pixel_probe --example pixel_scene
/tmp/astroterm-checks/bin/python scripts/checks/pixels.py --output /tmp/pixels-pty.json --capture-dir /tmp/pixel-captures
cargo run --release --example pixel_probe -- /tmp/pixel-prototype.png
cargo run --release --example pixel_scene -- /tmp/sky.png
```

The pixel PTY script tests forced Kitty/Sixel/iTerm2/half-block transport, pan/zoom, resizing with reported and
missing pixel sizes, automatic fallback after an unanswered query, character fallback after an oversized raster,
and quit/panic cleanup, including a panic inside an open synchronized update. Kitty checks also simulate accepted,
rejected and unanswered compression probes, including explicit Kitty selection, and decode the RGB payloads.
Kitty uploads precede synchronization; only placement and old-image deletion are synchronized. Other protocols
carry synchronized-update markers around their composed output; physical flicker still requires a
real-terminal check. Full graphics metadata is verified to be absent from the native text stream. The script
decodes an actual iTerm2 PNG, optionally saves it for inspection, and checks that pan/zoom changes the image.
It exercises input at 4 fps. Stage timings are now pixels themselves; use the debug panel or the headless prototype
to inspect them. The prototype measures drawing, text rasterization/encoding, buffer composition and writing to
memory at 1000×600. PTY startup measurements do not include an emulator's decoding/GPU display costs.

For physical checks in Contour, Rio or another terminal, start with:

```sh
make run -- --renderer pixels -i Tokyo -d 2025-03-01T11:00:00 -s 0 -C -m
```

Check stars, curves and readable labels/panel; pan with arrows, zoom with +/- and reset with 0. Resize repeatedly,
including narrow windows, and quit with q. Check for stale images, missing text or a broken shell/cursor afterwards.
Repeat with `--graphics-protocol sixel`, `kitty` or `iterm2` where supported, and `halfblocks` everywhere. The
metadata panel reports the chosen protocol and, for Kitty, the compression capability result. Kitty uses direct
image placement; Unicode placeholders are not required.
Also try `-g`, `-R`, `--disable-dynamic-names` and `--debug-frametimes`; use a tall terminal for the full timing list.
Report the emulator/version, protocol, flags and whether the issue occurs on startup, resize or subsequent frames.

```sh
uv venv /tmp/astroterm-checks
uv pip install --python /tmp/astroterm-checks/bin/python -r scripts/checks/requirements.txt
cargo build --release --bin astroterm --example terminal_probe
/tmp/astroterm-checks/bin/python scripts/checks/terminal.py
# Optional real-catalog startup/RSS measurement:
/tmp/astroterm-checks/bin/python scripts/checks/terminal.py --dataset datasets/athyg_40.csv.gz
cargo bench --bench frame -- --noplot
```

The PTY script supplies fixed time/location, sends repeated keys, resizes with and without metadata, checks
terminal attributes/cursor/alternate-screen restoration after quit and an intentional panic, and tests reported
pixel sizes plus the aspect-ratio fallback. It also checks the stellar fallback count on stderr and in the debug panel. It uses `pyte` to interpret terminal output. The script also measures
time to the first metadata-bearing frame, RSS after one second, and the process high-water RSS on Linux. It does
not drop the OS page cache. `--output path.json` records the results.

Run terminal measurements separately from CPU benchmarks. PTY/font emulation cannot certify how a real terminal
font displays ambiguous-width glyphs such as `⬤`, or physical keyboard repeat delivery. A real-terminal visual
check and macOS/Windows checks remain separate qualification steps. Sandbox signal restrictions can also affect
PTY resize tests; report the execution environment with the result.

Criterion uses the embedded catalog and fixed-seed 100k/2.5M synthetic catalogs. Synthetic catalogs preserve the
embedded bright stars and constellation endpoints and add a faint tail concentrated at magnitudes 9–13. Their
directions are uniformly distributed, so they are reproducible stress workloads, not substitutes for the real
Milky Way distribution. Setup/sorting is outside the measured loop. Updates use independent model caches and
advance by 1/24 simulated second per iteration, with/without refraction at thresholds 5/12 and with all stars.
The `project_draw` cases measure camera projection plus rendering of the threshold-selected prefix; they use 41×81 cells, 180°/10° facing views, thresholds 5/12, and constellations on/off. Sample size is 10,
warm-up one second, measurement two seconds (Criterion extends slow workloads). Results go to `target/criterion`.

For a real-catalog release measurement of loading, updates at `-t 5`, and update plus drawing (41×81, zenith,
Unicode/braille/color with constellations and dynamic names), run:

```sh
cargo run --release --locked --example catalog_probe -- datasets/athyg_40.csv.gz
```

The probe reports CSV loading and trajectory preparation separately, including singular/always-checked/endpoint counts and precision exceptions. Frame output distinguishes evaluated stars, interval-brightness candidates and currently drawable stars. It
excludes setup from per-frame measurements and does
not include terminal presentation or drop the filesystem cache. Run it on an otherwise idle machine.

Per-family refresh costs and cheap cached state evaluation have separate `models/` Criterion cases. For a focused
phase-2 measurement and the broader interpolation qualification sweep:

```sh
cargo bench --bench frame --locked -- '(embedded/update_geometric_t5|models/)' --noplot
cargo test --release --test simulation_pipeline --locked qualify_cache_intervals -- --ignored --nocapture
cargo test --release --test simulation_pipeline --locked cadence_tracks -- --nocapture
```

The sweep samples 12,000 epochs (including contemporary lunar cycles), both interval ends and both playback
directions. It checks angular and absolute position/velocity errors against direct evaluation, not a more accurate
physical ephemeris. Normal tests cover emission-time samples, tick transitions, independent refreshes, synthetic
anchors, model substitution, multiple observers/views and paused camera changes.

Phase-4 spatial-selection measurements use the real catalog when supplied, otherwise Criterion uses BSC5:

```sh
cargo run --release --locked --example catalog_probe -- datasets/athyg_40.csv.gz --matrix
ASTROTERM_BENCH_DATASET=datasets/athyg_40.csv.gz cargo bench --bench spatial --locked -- --noplot
/tmp/astroterm-checks/bin/python scripts/checks/terminal.py --dataset datasets/athyg_40.csv.gz --workloads
cargo test --release --test athyg_dataset --locked -- --ignored --nocapture
```

The matrix covers t5/180°, t12/10° and t12/180°, each with refraction and constellations on/off. The headless probe
uses a 41×81 canvas, fixed Tokyo/date/facing direction and paused simulation, with 20 warm-up and 80 timed frames;
it reports stage EMAs separately from the mean complete frame. Criterion separates observation and projection/drawing,
with additional depth-4/depth-6 queries. The optional PTY workloads use 55×160 cells, read the actual debug panel,
and time ten pan keys until changed Facing metadata appears. These latencies include PTY transport and pyte decoding;
they are upper-bound emulated responsiveness measurements, not physical display latency. They run sequentially.
The ignored real-catalog quantization audit checks both interval endpoints and each trajectory's closest approach.


Dataset/cache startup checks use the actual application loader (no terminal presentation):

```sh
cargo build --release --locked --example dataset_probe
# Use isolated OS data/cache folders on Linux; the first command downloads about 200 MB.
XDG_DATA_HOME=/tmp/astroterm-data XDG_CACHE_HOME=/tmp/astroterm-cache target/release/examples/dataset_probe athyg
XDG_DATA_HOME=/tmp/astroterm-data XDG_CACHE_HOME=/tmp/astroterm-cache target/release/examples/dataset_probe athyg
```

The first run verifies and installs the download, prepares the catalog and writes the cache. The second reports
`mapped=true`. A file path can be used instead of `athyg` to skip the download. To measure cold preprocessing,
use a fresh cache folder with an existing data file; record separately whether the OS page cache was dropped.
Unit tests cover checksum/semantic corruption, atomic concurrent writes, read-only folders, mapped lifetimes,
name/path selection, verified streams and download cleanup. Cached and uncached frames must remain identical.


Phase-6 qualification keeps the functional terminal checks and adds independent astronomy references
([reference README](../reference/README.md)). The model cadence sweep still has 12,000 epochs; orientation now
uses a 10-minute half-interval. The culling margin includes annual and diurnal aberration:

```sh
cargo run --release --locked --example aberration_bound
cargo test --release --locked --lib measure_aberration_cost -- --ignored --nocapture
```

The first samples 200,001 Earth velocities over the computational interval and adds the maximum WGS84 site spin;
production also expands the margin from the actual observer velocity every frame. The second isolates correction
arithmetic on one million directions. Neither substitutes for the final whole-frame optimization pass. Record
machine/build/frequency/concurrency conditions with performance results. Phase-6 Linux PTY checks passed; physical
fonts and other operating systems are not thereby qualified.


Runtime processing-cache checks and controlled headless comparisons:

```sh
cargo test --locked --test processing_cache
python scripts/checks/processing_cache.py --output /tmp/processing-cache-pty.json
cargo run --release --locked --example processing_cache
# Optional existing AT-HYG file; the probe's prepared catalog cache lives under the OS temporary directory.
cargo run --release --locked --example processing_cache -- /path/to/athyg_40.csv.gz
```

The probe compares enabled/disabled reuse at three fields of view and paused, forward, reverse and large-step
playback. Each row warms two frames, measures six, and reports stage means, refresh-frame means and hit counts.
The viewport is 1102×1102 sky pixels; encoding, labels and terminal presentation are excluded. Load time is excluded.
Run it without concurrent tests/benchmarks. These short comparisons do not establish sustained terminal throughput.
The production model windows are now ±30 seconds (planets), ±12 seconds (Moon), ±60 seconds (orientation); the
cadence test's boundary offsets have been updated accordingly. The full 12,000-epoch sweep remains opt-in.

Minimum-star rasterizer qualification (O1):

```sh
cargo test --locked --lib scene::pixels
ASTROTERM_DATASET=/path/to/athyg_40.csv.gz ASTROTERM_RASTER_OUTPUT=/tmp/astroterm-raster \
  cargo test --release --locked --lib compare_minimum_star_rasterizers -- --ignored --nocapture
```

The opt-in comparison alternates the original tiny-skia path and the minimum-star fast path on identical projected
scenes, asserts byte-for-byte image equality, and reports seven redraw measurements after a warm-up. It covers
thresholds 5/10 and fields of view 225°/115.2°/12.4°. Both paths really redraw each time; whole-scene cache reuse
cannot hide raster cost. Optional PNGs contain sky geometry only, before labels and metadata. Normal tests cover
all source/background channel values, transparent overlaps, clipping, integer coordinates through 4096, mixed
star radii and fallback on larger canvases. The shortcut has no computed mask cache; `--disable-cache` uses the
same direct blending arithmetic.


Observation optimization checks (O2–O4):

```sh
ASTROTERM_DATASET=/path/to/athyg_40.csv.gz \
  cargo test --release --locked --lib compare_catalog_access_paths -- --ignored --nocapture
cargo test --release --locked --lib measure_aberration_cost -- --ignored --nocapture
cargo test --locked --lib unit_aberration_matches_generic -- --nocapture
cargo test --locked --test processing_cache --test stellar_pipeline
```

The catalog comparison alternates full-record metadata / individual trajectory access with borrowed arrays on the
same selected indices; deferred labels are resolved only when needed in production. The aberration comparison uses
one million normalized directions and reports generic versus unit-input cost. Neither replaces the whole processing
probe. Its JSON now includes evaluated-star, skipped-correction and endpoint-only counts. Current-brightness
rejections may be rare near today; measure selection/publication overhead rather than assuming large O4 savings.


Compact draw-order checks (O5):

```sh
cargo test --locked --lib projection::
cargo test --locked --test stellar_pipeline --test processing_cache --test scene_snapshots
cargo test --release --locked --lib compare_compact_draw_order -- --ignored --nocapture
```

The opt-in comparison includes dependency-key collection, refresh checks, sorting, index extraction and result
storage. It alternates the former indirect comparator and compact records, discards two warm-ups, and reports
the median of ten measurements for refreshes and cache hits. Inputs are all BSC stars at magnitude ≤5 and
20k/250k synthetic sort entries; it loads no AT-HYG catalog and performs no astronomy or rasterization. It
checks exact index-order equality and reports retained scratch capacity in bytes. This isolates draw-order cost,
not whole-frame speed. Functional tests cover floating-point ties, dynamic names, membership changes and bypass.

### Single-frame execution report

```sh
python scripts/checks/singleframe.py target/release/astroterm --report-dir dev/singleframe-reports
cargo test --locked --offline --test pipeline_trace
```

The PTY check uses only embedded BSC at `-t 5`: ASCII, Unicode with frame timings/cache bypass, and pixels
through halfblocks, Sixel, iTerm2 and compressed Kitty. It verifies automatic one-frame exit, plain ordered
report after terminal restoration, repeated simulation calls, exact requested UTC, and one image placement.
It uses Python’s standard library on Unix; this validates transport, not physical display timing.
