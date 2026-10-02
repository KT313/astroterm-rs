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
pixel sizes plus the aspect-ratio fallback. It uses `pyte` to interpret terminal output. The script also measures
time to the first metadata-bearing frame, RSS after one second, and the process high-water RSS on Linux. It does
not drop the OS page cache. `--output path.json` records the results.

Run terminal measurements separately from CPU benchmarks. PTY/font emulation cannot certify how a real terminal
font displays ambiguous-width glyphs such as `⬤`, or physical keyboard repeat delivery. A real-terminal visual
check and macOS/Windows checks remain separate qualification steps. Sandbox signal restrictions can also affect
PTY resize tests; report the execution environment with the result.

Criterion uses the embedded catalog and fixed-seed 100k/2.5M synthetic catalogs. Synthetic catalogs preserve the
embedded bright stars and constellation endpoints and add a faint tail concentrated at magnitudes 9–13. Their
directions are uniformly distributed, so they are reproducible stress workloads, not substitutes for the real
Milky Way distribution. Setup/sorting is outside the measured loop. Updates are measured with/without refraction at thresholds 5/12 and with all stars;
drawing uses 41×81 cells, 180°/10° facing views, thresholds 5/12, and constellations on/off. Sample size is 10,
warm-up one second, measurement two seconds (Criterion extends slow workloads). Results go to `target/criterion`.

For a real-catalog release measurement of loading, updates at `-t 5`, and update plus drawing (41×81, zenith,
Unicode/braille/color with constellations and dynamic names), run:

```sh
cargo run --release --locked --example catalog_probe -- datasets/athyg_40.csv.gz
```

The probe reports loading separately from sky construction, excludes setup from per-frame measurements, and does
not include terminal presentation or drop the filesystem cache. Run it on an otherwise idle machine.
