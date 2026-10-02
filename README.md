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
- Planet orbital elements: [NASA Jet Propulsion Laboratory](https://ssd.jpl.nasa.gov/planets/approx_pos.html)

## License

MIT, see [LICENSE](./LICENSE). The original copyright notice of astroterm is kept there.
