# Working data and its owners

Start with the public types in `mod.rs`; `ApplicationState` is defined in `application/mod.rs`. It is created once
right after configuration validation, with every owner at its final type and an empty catalog; `replace_catalog`
installs the prepared catalog before the frame loop and is the only catalog mutation.
`pipeline.rs` borrows the `cache` fields separately for simulation, observation, projection and rendering. Algorithms live
in those processing modules; this folder owns their stored inputs, results and designated scratch buffers.

```text
ApplicationState
├── config                    validated settings and cache policy
├── persistent                data loaded once and never changed during the run
│   └── catalog               Arc<SkyCatalog>: immutable catalog data, installed once by replace_catalog
├── preparation               startup-only movement bounds, freed before the frame loop
├── cache                     everything recomputed from the persistent data and the simulated time
│   ├── sky                   observed objects ready for projection
│   ├── simulation            samples of planet, Moon and Earth-orientation models
│   ├── observation           selection, motion and correction caches
│   ├── projection            visible cells, draw order and projected geometry
│   └── rendering             character or pixel buffers, scene cache and transport data
└── timings                   timing records and optional bounded diagnostic history
```

The terminal session is a separate scoped guard: it owns the output writer and restores terminal settings before
reports print. It does not own scene data. `ProjectedSky` is a local borrowed view, not another stored copy or a
reference from one root field into another. Headless callers can instead own `ProjectionData` and borrow its view.

### Table registry

`tables/` lists every data table the root holds in one flat form, for memory debugging in any build. `Table` is one
original container (the complete star table, a Vec, a cache's stored value, an image, a canvas); `Tables` is an owner that visits its tables
with dotted paths such as `cache.observation.motion`. Each owner has one listing in `tables/owners.rs`: a
`list_tables!` line naming its fields and their cache `Group`, or a short hand-written `visit_tables` when a field
needs an adapter that borrows its complete owner. Add a field there when you add one to an owner; leaf impls for container types are in
`tables/leaves.rs`. A type implements `Table` or `Tables`, never both.

Row types declare their column names once, next to their struct, with `row_columns!(Name { a, b, c })` from the
foundation `rows` module; the compiler fills in the types and fails the build when the list and the struct differ.
Markdown-only labels and unit/index explanations live in `tables/labels.rs`. They do not rename Rust fields or
change storage. The comment below `row_columns!` maps named debug labels back to Rust fields; `labels.rs` also
maps anonymous tuple/array columns. Precise-motion previews expose seven named components of each original row.

`state.log_data(path, section)` writes a Markdown report: path, shape, used and reserved bytes, the cache policy and
metadata, then a Markdown table with typed column headers and the first and last ten rows. Every column stays
present; markup and pipes inside data are escaped. `Some(path)` appends to that file (created if missing); `None`
prints to stdout. Use a path inside the frame loop: the terminal session owns stdout and the alternate screen is active.


## Find an owner or a report

Other modules import through `crate::state::{ApplicationState, ObservationCache, ...}`. Supporting modules are
private; their layout is for navigation, not additional import paths.

| Folder | Responsibility |
|---|---|
| `application/` | The complete application root: `persistent` data, the `cache` stages and `timings`. |
| `processing/` | Simulation samples, observation/projection/scene caches, and their restricted borrowed views. |
| `rendering/` | Character/pixel frame buffers, terminal diff history and glyph storage. |
| `tables/` | The table registry and `log_data`: one flat listing of every table the root holds. |
| `memory/` | The inventory capture flow; collection and report helpers implement its bounded traversal and output. |

Shared records follow the same rule through `crate::model`: catalog representation is under `model/catalog/`,
celestial records under `model/celestial/`, projection/rendering records under `model/presentation/`, and validated
settings under `model/configuration/`. The algorithms remain outside these two ownership/data modules.

## Follow the indices

These are different positions in different arrays; they are not interchangeable:

1. A **catalog index** selects a prepared `StarStorage` row. It is not a CSV row, HR number or stable `StarId`.
2. A **working index** selects a `SelectedStar` and the matching motion/eligibility entries. Its `source_index`
   selects the catalog row. The working set includes required constellation endpoints.
3. **Correction selection** stores working indices retained for drawing or constellation geometry; its order
   determines the observed output list.
4. An **observed index** selects `cache.sky.stars`, after that correction selection.
5. A **visible index** selects projection's `(observed_index, Cell)` array.
6. A **draw-order position** selects an entry in the order array; that entry holds a visible index. Rendering follows
   dimmest-first magnitude, then ascending stable ID.

`Cell` is signed `(row, column)` with origin at the viewport's top-left. Depending on the renderer, the viewport
measures character cells or pixels. `TerminalViewport` also has an origin on the terminal screen; it is a different
type from `ProjectionViewport`.

## Catalog: prepared once, shared by reference

Definitions are under `model/catalog/records.rs`, `model/catalog/storage/`, `model/catalog/grid.rs` and `catalog/cache/`. `sky` prepares the
catalog; frame processing reads it. Catalog indices follow cell, conservative brightness bound and descending ID.

| Storage | Contents and readers | Lifetime |
|---|---|---|
| `stars` columns `u0`, `w` (N×3), `distance`; `precise_motions` side table | J2000 equatorial unit directions, normalized motion per Julian year (365.25 days), distance in parsecs; stellar propagation reads them through `columns()` or the `directions()`/`motions()` views | Immutable during the run; the `Arc` is swapped once at startup by `replace_catalog` |
| `stars` columns `magnitude`, `brightness_key` | Starting magnitude and conservative brightest magnitude; selection/propagation read them | Same |
| `stars` columns `id`, `name`, `display_color` | Stable u32 identity and label/color references; ordering, labels and appearance read them | Same |
| `grid.offsets`, coarse/fine caps | Catalog-region membership and conservative angular caps; region/brightness filtering reads them | Same; cap vectors are also built for cache-loaded catalogs |
| `figures.endpoints`, constellation figures/segments | Sorted catalog-index union and resolved endpoint pairs; endpoint merge and arc projection read them | Same |
| `names.text`, `names.boundaries`, `names.ascii_alternatives` | Shared label text with u32 boundaries and sparse ASCII variants; one-based star references survive sorting | Same; preparation-only deduplication map is dropped |

The per-star columns are declared once as `StarRow` (`model/catalog/storage/columns.rs`); `StarRowVec` owns one
vector per column and `StarRowSlice` borrows them for processing. The logger receives the original `StarStorage`,
not separate column views. Its `persistent.catalog.stars` entry reports all 8 runtime fields and sums actual column
lengths/capacities. Label boundaries, ASCII alternatives and precise motions have separate table entries; their bytes are not counted twice.

Prepared disk caches are read into a temporary byte buffer, validated and decoded into owned vectors. They use the
same representation as source-loaded catalogs. `CatalogArray` owns a vector; no catalog file mappings remain.
The byte snapshot is dropped before the loaded catalog is returned. Validation and atomic disk writes remain;
loading may temporarily hold both encoded bytes and decoded arrays. The catalog's Arc handles share one allocation,
counted once per feature-gated inventory. Observed skies borrow original names and share immutable definition sets;
projection keys retain handles to the same definitions, not copies of the figure or endpoint vectors.

`ApplicationState.preparation` exclusively owns the per-star movement bounds used to build/validate the grid.
Loading returns `PreparedCatalog { catalog, preparation }`; the disk-cache writer requires both together.
`free_preparation_only_data()` drops that owner after static frame preparation and before the clock starts.
Runtime rows remain a complete rectangular table, and existing catalog Arc identities do not change.
The cache format is version 3; older prepared caches are rejected and rebuilt from source normally.

Table previews sort map keys once per dump, format only the edge rows, and stop nested/text formatting at named
limits in `rows/`. Unknown sizes are labeled rather than reported as zero. The table listing is a quick view with
explicit partial counts; the feature-gated inventory remains the detailed ownership/deduplication report.

## Simulation: samples of physical models

`state/processing/simulation.rs` owns TT sample epochs/half-spans and family policy/version counters. `sky::update_simulation`
prepares samples; body/orientation evaluation and observer preparation read them.

| Field | Units/frame | Reset or replacement |
|---|---|---|
| `planets` | Samples of nine barycentric J2000 states in AU and AU/day, in body-ID order | Successful family preparation replaces the vector; planet version changes clear it |
| `moon` | Parent-relative lunar state; evaluation composes it with Earth's state | Lunar or orientation version changes clear it |
| `orientation` | Slow inertial-to-date rotation matrices | Orientation version changes clear it |

Zero-span policies clear the corresponding family at frame start, allowing exact within-frame samples. Request
vectors and the bounded sample vectors being constructed remain local until successful transfer into the owner.
Earlier families may already be updated when a later family fails; there is no whole-simulation rollback.

## Observation: selection, propagation and separate correction snapshots

`state/processing/observation.rs` owns the caches; `sky/observation/pipeline.rs` orchestrates the passes. Cache keys express actual
dependencies. Invalidation retains the old key/value but makes it unavailable through `Cache::value()` until refresh.

| Field family | Producer → readers; content/order |
|---|---|
| `catalog`, `prepared_classes` | Catalog preparation → stellar lookup; shared catalog identity and classifications by catalog index |
| `observer`, `light_time` | Reception geometry / emission-time preparation → selection and body sampling; observer state and TT epochs |
| `region`, `candidates`, `selected` | Region and brightness filtering → endpoint merge; cell IDs, then catalog indices |
| `working` | Endpoint merge → stellar motion/current brightness; sorted catalog indices plus drawable flags |
| `stellar`, `stellar_scratch`, `stellar_stats` | Stellar batches → motion output; per-catalog-index sample map, at most 1024 live scratch records, cumulative counters |
| `motion`, `eligible`, `corrections` | Propagation / current brightness / correction selection → observed output; parallel working-order directions/magnitudes/flags and retained working indices |
| `bodies`, `relative`, `illumination` | Emission sampling / observer subtraction / Moon lighting → corrections; body-order barycentric states, observer-relative AU vectors, phase data |
| `apparent`, `horizontal`, `refracted` | Aberration / rotation / optional refraction → next pass and projection; independent corrected-star/body-order snapshots |

Stellar geometric directions are J2000 unit vectors. Horizontal directions use East/North/Up. The body vectors also
carry distance where the pass requires it; they are not all unit vectors. Corrected directions never feed back into
catalog propagation. Hits restore the appropriate snapshot; refreshes calculate it once and keep the fresh output.

`stellar` does not evict entries during a run. Scratch clears after the refresh but retains capacity. Automatic
catalog-identity replacement clears catalog-dependent caches, classes and scratch while preserving observer and
light-time caches. Explicit catalog preparation resets the full observation owner and then builds classifications.
The two entry points deliberately have different lifecycles.

`cache.sky.stars` owns calculated records only, with catalog metadata borrowed through `ObservedStarView` on demand.
Correction selection clears/refills that vector; subsequent corrections write its positions. Planet/Moon fields are
updated in place. The legacy `candidate_indices` vector remains owned here for the uncached observation path.

## Projection: geometry backing plus a borrowed output view

`state/processing/projection.rs` owns these buffers; `projection/pipeline.rs` fills them. Rendering reads a `borrow_projected`
view of the completed fields. That view does not build a reference vector or clone body/arc/horizon geometry.

| Fields | Contents and use | Lifecycle |
|---|---|---|
| Immutable definition handle in constellation key | Read-only original definitions and their endpoint union | Shared; custom figures replace an immutable set through `set_figure_override` |
| `star_candidate`, `stars` | Exact observed-position/flag key, then visible `(observed_index, Cell)` result | Candidate is cleared on hit, moved into cache on successful refresh |
| `order_candidate`, `order`, `draw_order_scratch` | Exact visible magnitude/ID inputs, draw-order permutation, temporary sort records | Same key lifecycle; sort scratch retains capacity between sorts |
| `bodies` | Projected Sun/planet cells plus lunar geometry; hidden body records remain present | Refresh when the geometry key changes |
| `constellations` | Nested clipped arcs and sampled cell/pixel vertices | Same; computed independently of drawing toggle |
| `horizon` | View-dependent segments and orientation-label origins | Same |

Candidate storage is separate from committed storage. A refresh transfers the candidate, and the old committed key
drops normally. There is no cross-commit recycling. Equal-result refreshes do not advance the output generation.
A view's immutable borrows must end before its observation or projection backing can be mutated again.

## Rendering: scene snapshots, composition and terminal transport

`RenderingState::Pending` becomes `Chars` or `Pixels` when terminal setup succeeds. Resize updates backend geometry
and invalidates the appropriate scene data. `state/rendering/mod.rs` lists the fields; `terminal` and `scene` contain
all rendering algorithms.

| Owner/fields | Producer → consumer; units | Retention/reset |
|---|---|---|
| `SceneCache.prepared`, `named_candidates` | Catalog display preparation / pixel-key scan → appearance and labels; catalog and projected draw-order indices respectively | Prepared data retained; candidates rebuilt per key capture |
| Scene candidates and committed keys | Raster input capture → exact comparison; display values plus copied body/arc/horizon geometry | Hits clear flat vectors while retaining capacity; nested strings/arcs drop; refresh transfers candidate; failed pixel draws retain it for reset/retry |
| Scene pixel/character results | Raster passes → composition; RGBA pixels or canvas cells | Intentional image clone per output; character clone on refresh and restore on hit |
| Character `frame`, `presenter` | Sky/panel drawing → full-screen composition → diff writer | Resize replaces canvases and discards previous-frame snapshot; successful presentation updates previous |
| Pixel `frame_image`, `rgb` | Borrowed cached sky → sky/text composition → RGB conversion/encoding | Full frame consumed by conversion; RGB freed after upload encoding |
| Pixel `text`, `composed`, `encoded` | Text/image composition → serialization; ratatui cells or protocol handle | Text freed after painting/composition; composed/encoded results rebuilt per frame; encoded internals are opaque |
| Metadata `fields`, character `step_fields`, cache strings/notices | Metadata/timing formatting → panel/text | Field vectors refill in place; strings are rebuilt; notices retained as needed |
| `TextRasterizer` font/glyph map | Lazy glyph rasterization → text painting; glyph-keyed coverage bytes | Cell-size change, bypass or 512-glyph policy clears masks; font internals remain opaque |
| `upload`, `compressed`, `serialized`, `serialization_blank` | Kitty/image encoding → writer; bytes or terminal cells | Refilled only after previous writes/flushes complete; flat capacities retained |
| Timezone handle | Observer metadata lookup → formatting | Refreshed for changed observer; library internals are partial |

Retention is not a promise of allocation reuse. In particular, image and ratatui buffers currently rebuild each
frame. Retaining old capacity/data for inspection can raise steady or peak memory. Inventory snapshots do not prove
process peaks. Character raster output copies and the previous-frame character canvas remain intentional.

## Diagnostics and deliberate exceptions

`timings` owns ordinary timing arrays and optional trace events/details/inventories. Continuous memory diagnostics
retain startup, the latest completed frame, the current partial frame and bounded per-step aggregates, not a frame
history of unbounded length. See [the user guide](../../docs/memory-diagnostics.md) for limits and report meanings.
Collector path/dedup scratch is temporary; saved inventory descriptors carry sizes, not references to the subject.
Timing history counts these descriptors once without recursively revisiting their captured application data.

Not every local allocation belongs in the root:

- Startup arguments, cities, directories, parsers, catalog construction/validation/write scratch and capability
  replies remain local. They are not active-run buffer owners.
- Per-frame input controls, bounded light-time/sample requests, changed-figure endpoint fallback, short-lived label
  formatting/selection, body-sampling clones and construction results before cache transfer remain local.
- The rasterizer's temporary tiny-skia canvas transfers its pixels to the resulting image. Paths, paint scratch,
  image/compression internals, font tables and ratatui symbol allocations are library/local coverage exceptions.
- `TerminalSession` keeps its writer outside the root. The process-global timezone finder is an existing library
  exception; ordinary state refactoring does not duplicate it.
- Stateless/headless algorithms have explicit caller-owned result storage and local scratch. The main pipeline's
  ownership tree does not impose a process-global owner on those independent calls.

Inventory rows distinguish known payload, lower bounds and unknown storage. Shared allocations and mappings are
counted once per capture; mapped file length is neither heap use nor RSS. Buffer-operation events describe explicit
instrumented operations and permitted borrows, not every memory access, allocator call or physical byte transfer.

### One-use results and narrow inputs

Stellar samples retain direction, magnitude and the singular-fallback flag; distance ratio is only a local
calculation. Its removal can change a per-star diagnostic generation comparison, but these generations do not
control downstream processing. TTLs and exact-time reuse are unchanged.

Pixel text layout is freed after glyph painting (graphics) or cell composition (halfblocks). Glyph masks and
transport capacities remain reusable. The cached sky image is read-only; metadata paints only into the distinct
full-frame image. `scene::draw_pixels` returns a borrowed image, and `SceneCache::pixel_image` exposes that same
allocation after preparation. Headless callers needing an independent snapshot must explicitly clone it.
Halfblock encoding retains one scoped image copy because the installed library constructor consumes its image.

Constellation geometry helpers receive `&[Constellation]`, `&[ObservedStar]` and required settings, not the full
catalog. Definition sets have no writable public field access; custom sets are validated by
`sky::prepare_constellation_set` and explicitly installed. `None` restores default figures; an empty set hides them.
Observation continues including the base catalog's endpoint union; projection uses the active definitions.

Catalog preparation returns `io::Result`: source labels, text size and star-count limits are checked before publishing
the catalog. Internal IDs use u32 (maximum 4,294,967,295); AT-HYG IDs include skipped source rows. External Gaia
identifiers remain u64 during parsing and formatting. Prepared cache schema 7 stores u32 IDs, shared label
boundaries/alternatives and a one-byte display palette index; neither designation bytes nor source spectral/B-V
columns remain in the runtime catalog. Proper names and catalog identifiers have identical label
eligibility; text prefers the proper name. Logical labels with the same Unicode text and ASCII behavior share an
entry. A literal proper name and a designation with different ASCII behavior intentionally remain distinct.

Star colors are classified during catalog preparation from the original spectral letter and f32 B-V thresholds.
`StarColor` in model/ defines eight shared pixel/terminal palette entries. Rendering reads the catalog's validated
u8 index; there is no `PreparedScene`/`StarDisplay` allocation, separate name-presence array or scene-catalog Arc.
The raw classification inputs remain only in source loading. Per-star flags and precision references are removed; exception metadata has a separate empty skeleton.
The compile-time palette is static data, not per-run heap storage. Raster keys still retain their exact drawing
inputs; those intentional cache values are unchanged.

`SkyCatalog.star_exceptions` owns a sparse `Vec<StarException>` with final catalog row index, motion-fallback
boolean and one-based precise-motion reference. It must remain empty in this implementation, as must the reserved
`stars.precise_motions` payload. Source preparation still detects all exceptional trajectories before modifying
or storing them; it returns a descriptive Unsupported error. The completed preparation, cache read/write and
pre-frame boundary reject nonempty exception tables or orphaned precision payloads. A current-schema unsupported
cache is an error, not a silent source rebuild. Older cache versions still rebuild normally.

This deliberately suspends previously supported exceptional catalogs until sparse processing is implemented.
The numerical stellar model and runtime near-zero-distance fallback remain available, including outside the
preparation interval. Ordinary catalogs preserve their compact f32 motion inputs and behavior.

Prepared `magnitude` and `brightness_key` columns now each own u16 codes, decoded as `code / 1000.0 - 10.0`.
Raw magnitudes are validated as f64 in [-10.000, 55.535], then rounded to the nearest thousandth; halfway scaled
values round upward (fainter). Bounds use that decoded initial magnitude and the effective stored trajectory,
round downward, and are checked against decoding roundoff. Stationary stars keep constant brightness in both
the runtime model and the bound calculation. Current-time magnitudes stay unclipped f64.
The compact star columns occupy 41 bytes per star (column payload, excluding containers and side tables).

Derived out-of-range bounds clip to an endpoint. Bound code zero always passes early brightness pruning, even
below -10, so clipping cannot hide a star. `StarStorage` retains only two catalog-wide clipping counters; the
prepared cache persists and validates them. Console warnings appear once per load, and the TUI keeps a separate
notice alongside the date-range warning. The single `next_down` safety ULP below -10 is not counted as a real
clip. Raw range validation has no tolerance. Upper clipping is supported by the encoder but cannot arise from
the present whole-interval bound: the interval contains J2000, so its minimum is never above initial magnitude.

Table previews report u16 storage and show both code and decoded magnitude in the same cell; no second decoded
column is retained. `persistent.catalog.magnitude_clipping` exposes the inline counters. Quantization can create
new brightness ties and change near-threshold visibility or labels; draw-order ties still use stable IDs.
