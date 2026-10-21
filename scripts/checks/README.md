# Checks

## Structure (every commit)

```sh
python3 scripts/checks/foundation_imports.py       # --list prints the files it checks
```

The structural rules of `CLAUDE.md` ("Pipeline restructuring rules"), checked without reading Rust: the seven
step functions are called only from `src/pipeline.rs`; no `_with_progress` or `_with_times` twins; no
`super::super::super` paths; at most two module levels below `src/`, with the loader folders that still exceed it
listed in the script as exemptions pending the owner's decision. Exit code 1 on a violation.

## Terminal checks (pseudo-terminal, Linux)

```sh
cargo build --release --bin astroterm --example terminal_probe
python3 scripts/checks/singleframe.py target/release/astroterm --report-dir dev/singleframe-reports
python3 scripts/checks/pixels.py --output /tmp/pixels-pty.json --capture-dir /tmp/pixel-captures
```

`singleframe.py` runs `--debug-singleframe` with each graphics protocol and with the frame-time panel: one frame,
exit 0, the terminal restored before the plain-text report, the seven steps once each in order with their trace
details, one Kitty placement or one iTerm2 file. `pixels.py` drives the binary with forced Kitty, Sixel, iTerm2 and
half-block output, protocol detection replies, Kitty compression probes, pan/zoom/resize and quit, and the probe's
panic inside an open synchronized update; it decodes the RGB payloads. Both use the standard library only. A
pseudo-terminal validates transport and restoration, not what a physical terminal displays.

For a physical check in Kitty, Contour, Rio, WezTerm or another terminal:

```sh
make run -- -i Tokyo -d 2025-03-01T11:00:00 -s 0 -C -m
```

Check stars, lines and readable labels; pan with arrows, zoom with `+`/`-`, reset with `0`, resize, quit with `q`.
Repeat with `--graphics-protocol sixel`, `kitty`, `iterm2` or `halfblocks`, and with `-g`, `-R`,
`--disable-dynamic-names` and `--debug-frametimes`.

## Measurements

The frame and startup timings live in `dev/performance.md` (restructuring chunks 2, 7, 9 to 13). The warp
benchmark is `cargo bench --bench warp -- --noplot`; the per-step timing tests are the `#[ignore = "timing only"]`
tests in `src/regions`, `src/projection`, `src/points` and `src/present`, run with `--release --ignored
--nocapture`. `dataset_probe` measures the loader and the tables:

```sh
cargo build --release --locked --example dataset_probe
XDG_DATA_HOME=/tmp/astroterm-data XDG_CACHE_HOME=/tmp/astroterm-cache target/release/examples/dataset_probe athyg
```

Independent astronomy references and the probes that feed them are documented in
[scripts/reference/README.md](../reference/README.md).
