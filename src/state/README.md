# Working data and its owners

Start with the public types in `mod.rs`; `ApplicationState` is defined in `application/mod.rs`. It is created once
right after configuration validation, with every owner at its final type and an empty catalog; `replace_catalog`
installs the prepared catalog before the frame loop and is the only catalog mutation.
`pipeline.rs` borrows its fields separately for simulation, observation, projection and rendering. Algorithms live
in those processing modules; this folder owns their stored inputs, results and designated scratch buffers.

```text
ApplicationState
├── config                    validated settings and cache policy
├── catalog                   Arc<SkyCatalog>: immutable catalog data, installed once by replace_catalog
├── run
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


## Find an owner or a report

Other modules import through `crate::state::{ApplicationState, ObservationCache, ...}`. Supporting modules are
private; their layout is for navigation, not additional import paths.

| Folder | Responsibility |
|---|---|
| `application/` | The complete application root and the fields of an active run. |
| `processing/` | Simulation samples, observation/projection/scene caches, and their restricted borrowed views. |
| `rendering/` | Character/pixel frame buffers, terminal diff history and glyph storage. |
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
4. An **observed index** selects `run.sky.stars`, after that correction selection.
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
| `stars.u0`, `w`, `distance`, precise-motion exceptions | J2000 equatorial unit directions, normalized motion per Julian year (365.25 days), distance in parsecs; stellar propagation reads them | Immutable during the run; the `Arc` is swapped once at startup by `replace_catalog` |
| `magnitude`, `brightness_key`, `motion_bound` | Starting magnitude, conservative brightest magnitude and angular drift in radians; selection/propagation read them | Same |
| IDs, names/ranges, designations, spectral types, colors, flags | Stable identity and encoded display metadata; ordering, labels and appearance read them | Same |
| `grid.offsets`, coarse/fine caps, `always_checked` | Catalog-region membership and conservative angular caps; region/brightness filtering reads them | Same; cap vectors are also built for mapped catalogs |
| `endpoint_indices`, constellation figures/segments | Sorted catalog-index union and resolved endpoint pairs; endpoint merge and arc projection read them | Same |
| `names.text` | UTF-8 text block; labels resolve name ranges into it | Same |

A `CatalogArray` either owns its vector or holds a mapping handle and section range. All mapped sections can share
one `Arc<MappedCatalog>`; the map's logical extent is counted once. The catalog's other Arc handles also share one
allocation. Pointer deduplication applies within each inventory, never across captures.

The names and constellation figures cloned into `ObservedSky`, and the projection preparation copies of figures
and endpoints, are intentional retained copies. Owned name bytes can be copied; mapped names share the mapping.

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

`run.sky.stars` owns calculated records only, with catalog metadata borrowed through `ObservedStarView` on demand.
Correction selection clears/refills that vector; subsequent corrections write its positions. Planet/Moon fields are
updated in place. The legacy `candidate_indices` vector remains owned here for the uncached observation path.

## Projection: geometry backing plus a borrowed output view

`state/processing/projection.rs` owns these buffers; `projection/pipeline.rs` fills them. Rendering reads a `borrow_projected`
view of the completed fields. That view does not build a reference vector or clone body/arc/horizon geometry.

| Fields | Contents and use | Lifecycle |
|---|---|---|
| `prepared_figures`, `prepared_endpoints` | Figure copy and sorted catalog endpoint union used to recognize unchanged topology | Replaced on preparation; changed public figures use a local endpoint fallback |
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
| Pixel `sky_image`, `frame_image`, `rgb` | Raster clone → sky/text composition → RGB conversion/encoding | Rebuilt per frame and retained for inspection; conversions can consume/transfer allocations |
| Pixel `text`, `composed`, `encoded` | Text/image composition → serialization; ratatui cells or protocol handle | Rebuilt per frame; encoded internals are opaque |
| Metadata `fields`, character `step_fields`, cache strings/notices | Metadata/timing formatting → panel/text | Field vectors refill in place; strings are rebuilt; notices retained as needed |
| `TextRasterizer` font/glyph map | Lazy glyph rasterization → text painting; glyph-keyed coverage bytes | Cell-size change, bypass or 512-glyph policy clears masks; font internals remain opaque |
| `upload`, `compressed`, `serialized`, `serialization_blank` | Kitty/image encoding → writer; bytes or terminal cells | Refilled only after previous writes/flushes complete; flat capacities retained |
| Timezone handle | Observer metadata lookup → formatting | Refreshed for changed observer; library internals are partial |

Retention is not a promise of allocation reuse. In particular, image and ratatui buffers currently rebuild each
frame. Retaining old capacity/data for inspection can raise steady or peak memory. Inventory snapshots do not prove
process peaks. The cached raster output copies and previous-frame canvas remain intentional.

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
