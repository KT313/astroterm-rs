# Module map

Bottom-up. Every module exposes its symbols from its root `mod.rs`; callers use root paths.

| Module | Holds |
|---|---|
| `constants.rs` | tunable defaults, hold times, resource limits |
| `pixel_debug.rs` | optional channel checks and accumulated-light report (`pixel-debug` feature only) |
| `astro/` | time scales, coordinates, Earth orientation, VSOP87 planets, Moon, stellar motion, refraction, accuracy intervals |
| `catalog/` | BSC5, AT-HYG and Gaia parsers, MAST downloads and blob store, names and designations, magnitude coding, the prepared-catalog codec |
| `rows/` | the column-table macros the catalog and timing records use |
| `progress/` | the startup progress trait and `Report`, the one value that carries progress and timings through the loaders |
| `timing/` | `StepTimes`: smoothed step durations and the opt-in execution trace |
| `text/` | the fontdue text rasterizer used for labels and the panel |
| `model/catalog` | the data model: `Data` (definitions in `model/data.rs`: regions own groups; groups own star tables, simulation caches and tiles; constellation stars, bodies and names share this owner), the cube-map grid, speed-group rules, packed storage |
| `model/configuration` | `Config`, `View`, `RenderOptions`, `TerminalSettings` |
| `sky/catalog` | loading: resolve or download a dataset, prepare and cache it, build the sky tables |
| `solar_system/` | step 1, `simulate_solar_system` |
| `observation/` | step 2, `observe` |
| `selection/` | step 3, `select_regions` |
| `regions/` | step 4, `refresh_regions`: simulation caches, star appearance, tile geometry, group and region images |
| `projection/` | step 5, `project_stars_to_screen`: adaptive forward projection (`adaptive_warp.rs`), retained patch experiments (`hermite_warp.rs`, `coons_warp.rs`), shared mesh/coverage (`patch_warp.rs`, `triangle_rows.rs`), retained straight mapping (`tile_warp.rs`), shared texel sampling (`tile_sampling.rs`) and straight scanline coverage (`raster.rs`), frame buffers (`frame.rs`), shared projection geometry (`geometry/`) |
| `points/` | step 6, `draw_points` |
| `present/` | step 7, `present`, and `Display` (terminal session, protocol detection, frame layout) |
| `terminal/` | startup progress display, raw-mode session, key and resize input, graphics encoders and protocol detection |
| `controls/` | what a key does to the view and the clock |
| `metadata/` | the panel's text: local time, zodiac, Moon phase, location, speed, view, timings |
| `cli/` | argument parsing and validation, Bash completions |
| `time/` | binary-owned `StartupTiming` records setup, then is consumed into a fully initialized `TimeManager` with frame pacing and a simulation clock |
| `main.rs`, `pipeline.rs`, `helpers/` | the binary: startup, the seven-step frame loop, controls, panel lines, diagnostics and the frame benchmark |

Owned data at runtime: `Data` (immutable catalog columns beside mutable group/region caches), the `Frame`
(star layer, overlay, text rasterizer), the `Display` with its `Terminal`, and the `TimeManager` (which owns `StepTimes`). All are created in
`main.rs` or at the top of the frame loop and passed by reference to the steps.
