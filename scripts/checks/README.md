# Terminal and performance checks

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
