# astroterm-rs

A Rust port of [astroterm](https://github.com/da-luce/astroterm) (reference: the `feat/facing-view` branch), a
terminal star map showing stars, planets, the Moon and constellations for any date, time and location.

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

## Layers

Each module only depends on the ones above it:

| Module | Responsibility |
|---|---|
| `astro` | Julian dates, sidereal time, coordinate conversions, star/planet/Moon positions |
| `canvas` | In-memory cell grid (clipping, wide glyphs, braille merging) and line drawing |
| `projection` | Stereographic / equidistant projections and the `View` (zenith or facing, fov) |
| `catalog` | Embedded BSC5 star catalog, star names, constellation figures, cities, orbital elements |
| `sky` | Object model (`Sky`, `Star`, `Planet`, `Moon`) and position updates |
| `scene` | Drawing the sky, orientation aids and the metadata panel onto canvases |
| `terminal` | Raw-mode session guard, diffing presenter (crossterm), input |
| `controls` | Key bindings and their effect on the view and the simulation clock |
| `cli` | Arguments, validated `Config`, bash completions |

`src/main.rs` holds the processing flow: parse options → build the sky → per frame: poll input and apply controls,
update positions, draw (sky and metadata panel), present.

## Differences from the C version

New:

- Interactive controls (see [Keys](#keys)); the metadata panel shows the simulation speed and whether time is paused.
- With `--color`, stars are colored by spectral class (blue-white O/B in cyan, orange K in yellow, red M in red).
- The Moon includes the main perturbations by the Sun (about 0.1° instead of several degrees off), parallax, a
  phase computed from its actual elongation from the Sun, and an emoji lit on the side facing the Sun in the current
  view (the C version only mirrored it by hemisphere).

Fixed:

- Star labels are chosen at draw time instead of being erased from the star table while rendering.
- Braille constellation lines merge within canvas cells; no global 1024x1024 buffer.
- Constellation segments are clipped where they cross the edge of the view (both ends, also chords).
- Simulation time follows the wall clock (no drift when frames are slow).
- Datetimes are parsed as UTC without `mktime`, so local DST no longer shifts them.
- Mean anomalies wrap correctly for large negative values.
- The point directly behind an equidistant view has a fixed direction instead of a random one.
- The 14 BSC5 placeholder entries (no data) are no longer drawn as a bright star at RA 0 / Dec 0.
- Grid spokes are sorted with a valid comparator; Ctrl-C quits cleanly; the terminal is restored on panic.
- Metadata: the timezone abbreviation is the one in effect at the simulated date (from the system timezone database
  on Unix; elsewhere the UTC offset is shown); negative coordinates between 0° and -1° keep their sign
  (`-0° 30' 0.00"`).
- City lookup also ignores the case of non-ASCII letters.

## Development

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```
