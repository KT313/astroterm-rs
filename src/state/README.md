# Working data and its owners

Start with the public types in `mod.rs`; `ApplicationState` is defined in `application/mod.rs`. It is created once
right after configuration validation, with every owner at its final type and an empty catalog; `replace_catalog`
installs the prepared catalog before the frame loop and is the only catalog mutation.
`pipeline.rs` passes direct references to state fields, without destructuring the root. `current_view` starts
from `config.view` on each render-loop entry, and reset controls restore that original value. The live view also
appears in table dumps; its fixed-size storage is included in the root inventory's inline size. The pipeline borrows the `cache` fields separately for simulation, observation, projection and rendering. Algorithms live
in those processing modules; this folder owns their stored inputs, results and designated scratch buffers.

```text
ApplicationState
├── config                    validated settings and cache policy; view is the initial reset target
├── current_view              live camera changed by pan/zoom controls
├── persistent                data loaded once and never changed during the run
│   └── catalog               Arc<SkyCatalog>: immutable catalog data, installed once by replace_catalog
├── preparation               startup-only movement bounds, freed before the frame loop
├── cache                     everything recomputed from the persistent data and the simulated time
│   ├── sky                   observed objects ready for projection
│   ├── simulation
│   │   ├── solar_system      planet, Moon and slow Earth-orientation samples
│   │   └── stars             intrinsic motion, current magnitudes, classifications and scratch
│   ├── observer              reception geometry, light-time results and emission-time bodies
│   ├── selection             conservative regions, candidates and constellation endpoints
│   ├── observation           brightness eligibility and apparent-direction corrections
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
with dotted paths such as `cache.simulation.stars.regions`. Each owner has one listing in `tables/owners.rs`: a
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

Binary-only supporting functions are exported by `src/helpers/mod.rs`. Its private `startup/`, `frame/` and
`diagnostics/` folders group setup, individual frame stages, and optional logging/reporting. `main.rs` still owns
the terminal lifetime and calls `run_render_loop()` directly; `pipeline.rs` keeps the processing order visible.

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
2. A **working index** selects a `SelectedStar` and the matching eligibility entries. Its `source_index`
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
catalog; frame processing reads it. Catalog indices follow simulation region, conservative brightness bound and descending ID.
Spatial regions exclude endpoints; the final region contains exactly the unique figure endpoint union.

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

`ApplicationState.preparation` exclusively owns the per-star movement bounds retained for prepared-trajectory validation.
Region assignment now depends only on stored directions, not these bounds.
Loading returns `PreparedCatalog { catalog, preparation }`; the disk-cache writer requires both together.
`free_preparation_only_data()` drops that owner after static frame preparation and before the clock starts.
Runtime rows remain a complete rectangular table, and existing catalog Arc identities do not change.
The cache format is version 3; older prepared caches are rejected and rebuilt from source normally.

Table previews sort map keys once per dump, format only the edge rows, and stop nested/text formatting at named
limits in `rows/`. Unknown sizes are labeled rather than reported as zero. The table listing is a quick view with
explicit partial counts; the feature-gated inventory remains the detailed ownership/deduplication report.

## Frame order and restricted inputs

The binary runs solar-system simulation → observer preparation → star selection → stellar simulation → observation
corrections → projection → rendering. The first two stages finish every fallible body lookup before stellar work.
The main loop passes direct field references; none of these algorithms receives the application root.

`SimulationCaches` groups two independent owners. `SimulationState` keeps its existing type name and owns only
solar-system samples under `cache.simulation.solar_system`. `sky::update_solar_system` prepares reception and
requested emission coverage. Planetary states are barycentric J2000 AU/AU-day; Moon samples are Earth-relative
and composed with Earth at the requested epoch. Slow orientation uses TT, while observer preparation applies
current UT1 spin. Managed requests refresh on time/location changes; zero-duration policies permit same-time
reuse, while explicitly disabled families bypass it. Sample selection
remains first-covering, history remains bounded, and earlier families can remain updated if a later family fails.

`ObserverPreparationCache` owns reception geometry, light-time results and final emission-time `BodySamples`.
`sky::prepare_observer_inputs` runs geometry, two light-time request iterations, and final body sampling. The
reception observer remains unchanged while emission requests refresh the independent solar-system owner.
`PreparedBodies` borrows only completed body data, and its constructor checks the observer matches the sample key.
Getters never invoke an ephemeris. Synthetic callers may still prepare a custom observer and sample its supplied
emission epochs without rebuilding its geometry.

`StarSelectionCache` owns spatial selection, regional brightness/validation slots, scalar selection statistics
and the endpoint-aware `working` records. There are no combined candidate or validated index vectors.
Regional slots keep only a qualifying catalog prefix `(start, end)`: brightness
keys depend on threshold and interval status, validation keys on that region's prefix generation. Only requested
regions are checked; offscreen entries remain retained. Equal prefixes preserve versions even if thresholds change.
Spatial selection keeps its fixed
0.25° drift allowance and aberration/refraction/quantization margins. The final simulation region exclusively
owns the unique constellation endpoints and is always requested, even with lines disabled. Endpoint dots still
pass brightness and projection checks; faint endpoints can support lines without becoming drawable dots.
Outside the interval all regions are requested. Fast non-endpoint stars still use fixed catalog regions and can
be missed after large drift. `SelectedStars` borrows both the requested region IDs (before brightness filtering)
and the working rows, along with catalog identity, requested epoch and source generation. Its `ranges()`
iterator yields `(region_id, start, end_exclusive, validated_generation)` directly from the saved slots.
The working-set builder expands these ranges as an iterator while merging endpoints, without collecting indices.

`requested_sources` retains one `(region_id, validated_generation)` tuple per requested region. Its allocation
is reused; `selection_revision` changes only when this ordered metadata changes. `request_revision()` exposes
that request identity separately from the value-based working-row generation. Adding an empty region can
change the former without changing any star rows. Stellar results check both preparation provenance and
this requested-selection revision so unprepared regions cannot enter an already-published result view.

`StellarSimulationState` owns `regions`, `refresh_regions`, `prepared_classes`, bounded `stellar_scratch`, regional
statistics/revision, one reusable `region_output_work` vector and scalar publication metadata. `prepare_stellar_catalog` takes start TT explicitly
and initializes every slot empty/invalid before the loop. Lazy headless setup uses the first requested TT.
Each requested region checks one timestamp/invalidation flag, then refreshes its complete catalog range when
needed. The constellation region follows precisely the same numerical/cache path. No per-star cache map remains.
Numerical passes borrow trajectory/magnitude columns, evaluate in f64, and append samples in catalog order.

The default/max stellar TTL is 864000 simulated seconds in `constants.rs`; existing config overrides may
shorten it. Values are held, not interpolated or accuracy-qualified. Both direction and magnitude can remain at a
region's calculation epoch even after simulation time changes. Age is absolute in TT, including reverse playback;
the exact boundary is reusable, and hits never slide timestamps. `--disable-cache` and disabled groups
recalculate all requested regions. Zero TTL permits reuse at exactly the same time. Model formulas are unchanged.

Regional freshness is checked before publishing `last_request`: selection owner/working generation, requested-region
revision, requested TT and regional-results revision. Failed preparation clears readiness. There is no combined
selected-motion cache or slice. `StellarResults::selected_samples()` walks the ordered working rows and regions,
yielding `(catalog_index, &StellarSample)` from the original allocations with one validity check per region.
`selected_count()` and `fallback_count()` expose scalar counts; fallback counting runs only when its selected
rows or regional sample inputs change. Every dot/line endpoint reads its sole owning region.

Refreshes calculate complete samples in `region_output_work`. `Cache::store_reusing` compares old and completed
values once, swaps allocations, then clears the displaced vector without shrinking it. Initial population and
capacity growth still allocate; the extra work buffer keeps capacity between frames. Allocation identity may move
between region owners. Empty regions use allocation-free empty vectors so they cannot retain a useful work buffer.
The original payload stays intact but invalid if calculation fails before commit. Generations change only when
values differ. Regions stay until catalog replacement; there is no eviction or generic allocation pool.

`StellarResults` borrows intrinsic results and their matching selection. Constructors check catalog identity,
selection identity/revision and requested TT; observation also checks the observer's epoch. The requested epoch
is not a claim that every region was calculated then. Borrowing the view prevents mutation of its owners.
The table logger/inventory report original regional owners, retained work capacity and bounded previews without
reconstructing a flattened sample array.

`ObservationCache.regions` retains independent brightness flags, correction membership and stellar aberration
for each catalog region. Keys contain that region's upstream versions plus threshold or observer velocity where
needed; none scans individual stars to decide reuse. Correction records retain catalog indices. Solar-system
aberration has a separate small `body_apparent` cache. There are no combined eligibility flags or correction-index
lists. `layout_sources` retains ordered region/correction/sample versions. Only changes rebuild the small
`regional_output` spans and summed `layout_stats`; unchanged frames never scan individual retained stars here.
Catalog indices stay in original regional records; spans address final direction buffers, including empty regions.

`ApparentDirections` is a local read-only view over requested region descriptors, original regional direction
slices and the original body-apparent cache. It is created after successful regional/body aberration; no view is
stored inside state. Region iteration verifies membership/result versions and direction counts before supplying
one slice to its star loop. Horizon rotation receives this view beside separate writable horizontal metadata,
cache and reusable work. It writes transformed directions directly; it never uses the summary's old positions
as apparent inputs. Stars and Moon retain the direct path's vector lengths; planets use the same normalization.

There is no combined apparent snapshot, apparent assembly or apparent restoration into observed output.
The Stage-4 correction-membership bridge is also gone. `horizontal_sources.regions` stores ordered
`(region_id, start, end, selection_generation, apparent_generation)` tuples, plus a separate body generation.
Its reused-capacity snapshot advances `revision` only when these dependencies change. `(revision, rotation)`
keys the horizontal cache, whose result generation still depends on exact direction equality. Owner/catalog
replacement resets the metadata with observation storage. It is region-sized metadata, not a direction buffer.

Regional/body apparent inputs stay unchanged. Horizontal and refraction misses fill `horizontal_work` or
`refraction_work`, then `Cache::store_reusing_pair` compares once and swaps complete result allocations into the
cache. Displaced vectors clear without shrinking. Hits return the original cache result without capture, restore,
or a per-star loop. Initial population/growth still allocate; retained spare capacity can increase memory.

`sky::observe_cached_regions` returns a local `RegionalObservation` containing `ObservedSkyView`. Its star
accessors combine original regional correction records (catalog index/draw flag), stellar samples (magnitude),
and selected horizontal/refracted directions. It owns no star array. `ObservedStarView.state` either borrows an
owned row or holds a small resolved value on the stack; neither variant owns heap memory. Plain regional cache
record definitions live in model so this view never imports state; ObservationCache still owns their allocations.

Production `cache.sky` contains scalar metadata and small body/illumination scratch; `cache.sky.stars` stays empty.
Its body-position fields are intermediate values, not the final sky: read final body positions through the view.
`published` validates stellar owner, selection/region request, result revision, epoch and catalog identity before
`observed_view` can expose cached directions. `use_refraction` selects the completed result. Invalidation or a
new unfinished observation clears publication; immutable views prevent their owners from changing underneath.
The loop reborrows this view after projection's mutable stage before rendering; no sibling references live in state.

The explicitly owned `observe_cached_sky` API and `ObservedSkyView::materialize()` still produce normal owned
snapshots for compatibility/export. That operation is linear and labeled `Observed output materialization`;
the production loop never calls it. Direct/caller-editable projection remains supported. Generic projection
entry points accept either a borrowed sky view or `&ObservedSky`; use `&*sky` to reborrow a mutable reference.

Regional projected cells retain a compact `(region_slot, observed_index)` address, not a cached observed-star
record. The slot indexes current region descriptors; the row indexes final directions. This avoids per-star
region searching in projection/rendering and retains the previous address footprint on 64-bit targets. Region
order and constellation-last drawing remain unchanged. Render/projection views borrow original data; table logs
and inventories report owners/work capacity and never reconstruct a star table for diagnostics.

Cache generations still change only when values change. View controls invalidate selection and projection;
resize only invalidates projection. Catalog identity changes reset selection, intrinsic samples and corrections,
including equal-content distinct catalog allocations. Automatic stellar replacement classifies on demand until
explicit preparation is requested. Independent reception, light-time and body caches survive a catalog change;
body-cache retention is newly possible because it no longer belongs to the catalog-dependent owner. Root
`replace_catalog` resets all affected owners and clears observer preparation as part of its fresh-run installation.
Explicit stellar preparation replaces the stellar owner; its new identity also forces dependent corrections to
refresh on their next use. `Obs` metadata aggregates observer, selection, stellar and correction cache statistics,
preserving its historical combined meaning.

Missing body coverage retains the previous cache value but invalidates it; no new observed sky is published.
Later stages do not run, and failure diagnostics are still printed after terminal restoration. Headless
`observe_sky`/`observe_sky_candidates` are compatibility coordinators in `sky/pipeline.rs`: they call separate,
cache-free selection/simulation/correction algorithms and honor caller-provided observer geometry.

`cache.sky.stars` owns calculated records only, with catalog metadata borrowed through `ObservedStarView` on demand.
Correction selection clears/refills that vector; subsequent corrections write its positions. Planet/Moon fields are
updated in place. The legacy `candidate_indices` vector remains owned here for the uncached observation path.

## Projection: geometry backing plus a borrowed output view

`state/processing/projection.rs` owns these buffers; `projection/pipeline.rs` fills them. Rendering reads a `borrow_projected`
view of the completed fields. That view does not build a reference vector or clone body/arc/horizon geometry.

| Fields | Contents and use | Lifecycle |
|---|---|---|
| Immutable definition handle in constellation key | Read-only original definitions and their endpoint union | Shared; custom figures replace an immutable set through `set_figure_override` |
| `regional_stars` | One small dependency key and retained `(catalog_index, Cell)` vector per region | Refresh only requested regions whose dependencies changed |
| `regional_orders` | Regional magnitude/ID sorted records with region-local observed-row offsets | Membership versions guard these offsets; camera and position-only changes do not invalidate order |
| `regional_cells`, `regional_ranges`, `assembled_for` | Directly drawable cells, one start/end range per region, and small assembly dependency records | Reassemble only when requested regions, row ranges, cells or order versions change |
| `regional_cell_work`, `regional_order_work` | Replacement cells and sort records shared sequentially across regions | Clear/reserve, calculate, compare once, then swap with completed result; retain displaced capacity, including across owner/catalog resets |
| `region_cell_scratch` | Optional screen cells indexed within one observed region | Reused across regions and frames; empty after assembly, capacity bounded by the largest observed region encountered |
| `star_candidate`, `stars` | Exact observed-position/flag key and visible output for the caller-editable headless fallback | Candidate is cleared on hit, moved into cache on successful refresh |
| `order_candidate`, `order`, `draw_order_scratch` | Exact visible magnitude/ID inputs, draw-order permutation, temporary sort records | Same key lifecycle; sort scratch retains capacity between sorts |
| `bodies` | Projected Sun/planet cells plus lunar geometry; hidden body records remain present | Refresh when the geometry key changes |
| `constellations` | Nested clipped arcs and sampled cell/pixel vertices | Same; computed independently of drawing toggle |
| `horizon` | View-dependent segments and orientation-label origins | Same |

The frame loop calls `project_cached_regions` with the immutable observation token. Projection keys depend on
regional membership/apparent versions, horizon/refraction, camera and viewport. Regional sorting depends on
membership and stellar versions. Each ordinary region is drawn dimmest-first, then the constellation region is
drawn last. Cross-region pixel/cell overlaps follow that region order; no global merge, merged-order array or
catalog-wide visibility lookup remains. Assembly resolves cells using reusable region-local scratch and emits
directly drawable `(RegionalStarIndex, Cell)` rows. Regional order offsets are valid only under their membership
key; they never store shifting whole-frame row offsets.

Regional refreshes leave the previous result intact but invalid until replacement calculation and sorting finish.
One work vector per result type exchanges allocations with each refreshed cache; initial population and capacity
growth still allocate. Hits do not fill or sort work. Spare capacity can raise retained memory; this is allocation
reuse, not a memory-reduction claim. Interrupted work is cleared on retry and never exposed as a completed result.

Labels independently select the global brightest five by comparing up to five eligible candidates from each
region. Pixel candidates still pass the full-footprint check. The winners occupy a fixed stack array; regional
ranges let selection skip the rest of the stars. The opacity floor still runs once after all star drawing. New region
slots are initialized lazily on first use and reset on catalog/owner replacement. Unrequested results are retained;
these new caches use exact dependency invalidation, not the stellar ten-day TTL.

The existing `project_cached_sky` API retains exact input comparisons for arbitrary caller-edited skies. It shares
body/figure/horizon geometry caches with the regional path, but has separate stellar backing. Switching paths
selects the appropriate backing before `borrow_projected`. Metadata counters remain incremental; inventories
and table reports aggregate regional allocations, including offscreen results, without thousands of output rows.

For the exact fallback, candidate storage is separate from committed storage. A refresh transfers the candidate, and the old committed key
drops normally. There is no cross-commit recycling. Equal-result refreshes do not advance the output generation.
A view's immutable borrows must end before its observation or projection backing can be mutated again.

## Rendering: scene snapshots, composition and terminal transport

`RenderingState::Pending` becomes `Chars` or `Pixels` when terminal setup succeeds. Resize updates backend geometry
and invalidates the appropriate scene data. `state/rendering/mod.rs` lists the fields; `terminal` and `scene` contain
all rendering algorithms.

Production pixel rasterization receives a sealed `RenderProjection` from `borrow_render_projection`. The handoff
validates the completed view, source owner/catalog, region membership/motion and projection keys before borrowing
geometry. Its public sky accessor is read-only; editable `ProjectedSky` callers still use exact `draw_pixels` keys.
Raster keys contain the projection owner/source-replacement revision, body/constellation/horizon generations,
per-region descriptors (including motion/magnitude versions), cell/order generations and rendering settings.
Thus a paused hit builds/compares only small region metadata, never star records or copied geometry. Source
replacement increments a checked revision; a new projection owner has a distinct identity. Failed projection
cannot publish this handoff. The key is dependency-only; time alone neither forces nor proves a redraw.

On a trusted miss, `SceneCache.pixel_inputs` prepares filtered drawing records once, and rasterization borrows
those records. It clears them after success, retaining capacity; a failed draw is reset on retry. Production and
exact keys share one image cache but cannot match each other. Pixel result generations still advance only when
actual stored pixels change; `pixel_generation()` exposes this for downstream reuse. Raster bypass and explicit
invalidation still force drawing. Text, image conversion and upload are unchanged by this stage.

Star labels select globally from at most `DYNAMIC_NAME_COUNT` eligible candidates at the bright end of EACH
region; no label-candidate buffer is retained.
Sun, planet and Moon labels are independent of that limit.
Projected views carry their FOV so pixel-star brightness can increase when zoomed in. The pixel scene key includes
this value even when projected cells are unchanged; character scene keys do not depend on this display adjustment.
Pixel labels skip stars omitted by the four-pixel boundary check, walking backward only until enough eligible
stars are found. This needs no heap allocation; a view with many edge stars may require inspecting more than N.

| Owner/fields | Producer → consumer; units | Retention/reset |
|---|---|---|
| `SceneCache.star_layer` | Prepared pixel-star inputs → custom RGB/opacity blend → opacity floor → opaque sky composition; row-major `StarPixel` values | 16 bytes per pixel (f32 RGB + opacity); reset and reused on refresh, unchanged on hit; capacity retained across resize |
| Scene candidates and committed keys | Trusted region/version metadata OR exact input capture; only exact callers copy per-star and body/arc/horizon geometry | Hits clear flat vectors while retaining capacity; nested strings/arcs drop; refresh transfers candidate; failed pixel draws retain it for reset/retry |
| `SceneCache.pixel_inputs` | Trusted raster miss → drawing; accepted screen coordinates, magnitude and RGB | Empty after success, capacity retained; no per-star writes on hits |
| Scene pixel/character results | Raster passes → composition; RGBA pixels or canvas cells | Pixel image borrowed; character clone on refresh and restore on hit |
| Character `frame`, `presenter` | Sky/panel drawing → full-screen composition → diff writer | Resize replaces canvases and discards previous-frame snapshot; successful presentation updates previous |
| Pixel `frame_image`, `rgb` | Borrowed cached sky → sky/text composition → RGB conversion/encoding | Kitty retains reusable RGBA work and completed RGB; other graphics protocols consume RGBA during encoding |
| Pixel `text`, `composed`, `encoded` | Text/image composition → serialization; ratatui cells or protocol handle | Text retained with dependencies; composed/encoded results rebuilt for non-Kitty output; encoded internals are opaque |
| Metadata `fields`, character `step_fields`, cache strings/notices | Metadata/timing formatting → panel/text | Field vectors refill in place; strings are rebuilt; notices retained as needed |
| `TextRasterizer` font/glyph map | Lazy glyph rasterization → text painting; glyph-keyed coverage bytes | Cell-size change, bypass or 512-glyph policy clears masks; font internals remain opaque |
| `upload`, `compressed`, `serialized`, `serialization_blank` | Kitty/image encoding → writer; bytes or terminal cells | Refilled only after previous writes/flushes complete; flat capacities retained |
| Timezone handle | Observer metadata lookup → formatting | Refreshed for changed observer; library internals are partial |

Kitty composition checks `frame_key` before initializing or touching pixel buffers. Its owner-local sky/text
versions, dimensions, screen/sky placement, physical font size, text-cell size and background cover every image
input. `rgb_version` is ready only after complete conversion; failed preparation invalidates it. The conversion
copies RGB channels without multiplying alpha, because drawing already blends into an opaque background.
`encoding_key` combines that RGB revision with dimensions, compression capability, tmux and target image ID.
Encoding reuse borrows the existing upload/compression buffers; alternating image IDs correctly require new
commands. Both result paths require Raster and RasterAssets reuse. Retained RGBA/RGB capacity can increase
steady memory; no peak-memory reduction is claimed.

Text and sky are prepared before full-image composition. Consequently the live timing panel shows the current
raster timings and the previously measured composition/conversion timings; those later steps have not executed
when this frame's metadata is collected. They still update on every executed pass.


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

Pixel text layout remains available after glyph painting (graphics) or cell composition (halfblocks). Its
dependency checks skip unchanged layout work; glyph masks and transport capacities remain reusable. The cached sky image is read-only; metadata paints only into the distinct
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


Tunable cache durations, grid settings, control/view defaults, drawing limits and diagnostic caps live in
`src/constants.rs`, below all application layers. Modules import constants directly from `crate::constants`.
Physical constants, formula coefficients, protocol/file-format definitions and validated fast-path parameters
remain next to their algorithms. Solar-system cache configuration and default sample windows share the same
seconds constants. Prepared-catalog fingerprints include the storage-affecting settings by value, so changing
an unrelated display limit or TTL alone does not force a catalog rebuild.

### Solar-system request and sample work buffers

`cache.simulation.solar_system` is an always-requested group, separate from the stellar region grid.
`group` records the request/completion keys, calculation time, invalidation flag and request token. The token
tracks preparation provenance, not numerical equality. Time, observer location, observer-cache identity,
model versions and sample policy determine reuse; camera changes do not. The default group TTL is zero:
unchanged paused frames reuse completed results, while any advancing/reversed time refreshes the group.
Explicitly disabled caching still refreshes paused frames.

Reception and emission requests share that active preparation. Completion is published only after both
light-time iterations and final body sampling succeed. Per-family interpolation spans retain their existing
meaning within the request. Standalone `update_solar_system` remains a lower-level sampler; full frame callers
use `begin_solar_system_frame` followed by `prepare_observer_inputs`.

`planet_work`, `moon_work` and `orientation_work` hold replacement sample lists during preparation. On success,
each swaps with its corresponding saved sample list and is cleared without shrinking. Failures preserve the
previous saved list from that family call. Both allocations retain capacity; inventory/table reports include
the scratch storage. Unchanged lists and completed paused requests avoid rebuilding and copying.

### Retained pixel labels and text

`PixelState.text_cache` owns at most `DYNAMIC_NAME_COUNT` label descriptions (text, color and cell origin),
regional label dependencies, and the last displayed text inputs. `text` owns the single completed cell grid;
`text_version` marks it ready only after layout finishes. No views or catalog strings are retained by reference.

Protected projection owner/catalog identity, regional membership/magnitude/cell/order generations, threshold,
dynamic-name setting and text mapping govern label reuse. Direct editable projections refresh conservatively.
Text reuse compares the label revision, body/orientation positions, layout, actual visible metadata rows and
notices. Metadata outside terminal rows is ignored; different text within a visible row can conservatively
refresh even if its changed suffix is clipped. Timing metadata remains live. The `RasterAssets` policy controls
both caches; bypass refreshes them. Resize invalidates readiness.

A miss resets and refills retained text cells in the original paint order. A hit skips candidate selection,
name formatting and cell writes. Pixel/halfblock consumers borrow the finished text without releasing it.
Retained capacity raises steady memory relative to the old temporary grid; no lower-memory claim is made.


### Completed rendering versus displayed output

`RenderResultVersion` distinguishes missing/invalid output from a completed owner-local revision. PixelState
owns the label descriptions/dependencies, retained text grid, RGBA work, completed RGB, frame/encoding keys and
`displayed_key` / `display_valid`. The displayed key is the last successfully submitted RGB revision plus screen,
dimensions and transport settings. `display_valid` becomes false before writes and on resize/clearing/configure;
failed writes preserve the old key and image ID. It becomes true with the new key only after upload and swap
writes/flushes succeed. An unchanged valid display skips encoding, upload, swap and ID advancement.

`Renderer::render_prepared_frame` returns `RenderOutcome` for pipeline completion diagnostics. The editable
`render_frame` API keeps its unit result; internally it still records successful submissions. Timing records count
actual successful presentation calls and display reuse separately, instead of inferring success from entering
an I/O timer. Non-Kitty renderers retain their normal presentation behavior. Known invalidations are handled;
out-of-band terminal image loss is not detected. Live timing metadata still updates and can require new output.
