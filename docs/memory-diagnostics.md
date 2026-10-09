# Inspect working data and memory operations

`--debug-memory` reports which buffers the application keeps, their payload sizes, and the instrumented operations
performed by each pipeline step. It helps answer **“what data does this step work on?”** It is not a heap profiler or
a measurement of physical RAM traffic. Normal builds omit the collection and storage engine; the lazy hook API compiles to no-ops.

## Quick dump without the feature

Pass `--debug-log-data` to append full-state dumps at these checkpoints, relative to the current directory:

| File | Checkpoint | Frequency |
| --- | --- | --- |
| `tmp/tables.md` | After catalog/terminal setup, before preparation | Once per run |
| `tmp/after-preparation.md` | After static precomputations, before freeing startup data | Once per run |
| `tmp/after-preparation-cleanup.md` | After freeing preparation-only bounds | Once per run |
| `tmp/after-projection.md` | After simulation, observation and projection | Every frame |
| `tmp/after-rendering.md` | After rendering and presentation | Every frame |

Missing folders are created. This works in every build; combine with `--debug-singleframe` for one dump per file.
Logs append on subsequent runs. In-frame dumps add to frame time and can produce large files during continuous
runs. Omit the flag for benchmarks without table logging; no log file or folder is created.
Additional pipeline dump points can use
`if state.config.debug_log_data { ... }` with `state.log_data(...)` or the binary's `log_pipeline_data` helper.
The helper accepts a destination path, for example
`log_pipeline_data(state, "tmp/startup.md", "stage-renderloop-start")?`; missing parent folders are created.
Use different paths to keep separate dumps, or the same path to append sections to one file.

Any build can write the original tables held by state (catalog, caches, rendering buffers) with their shape,
used and reserved bytes, cache policy and metadata, column names and types, and the first and last ten rows:

For callers using the explicitly owned observation API, an observed-star table renders as:

### cache.sky.stars

**Shape:** `[1319]` · **Used:** 61.8 KiB · **Reserved:** 61.8 KiB

| Row | source_index: usize | drawable: bool | magnitude: f64 | position: Vector3 |
| ---: | --- | --- | --- | --- |
| 0 | 1 | true | 4.67 | Vector3 { x: 0.567, y: -0.798, z: 0.199 } |


From code:

```rust
state.log_data(Some(Path::new("/tmp/astroterm-tables.md")), Some("after frame 3"))?;  // append a section
state.log_data(None, None)?;                                                            // print to stdout
```

Use a file path when the frame loop is running: the terminal owns stdout and the alternate screen is active.
The star catalog is one `persistent.catalog.stars` Markdown table with all 8 runtime columns. Each table has a heading,
memory details and typed column headers; each cell has a bounded preview. Label boundaries, sparse ASCII alternatives and precise motions are separate side tables because they
have different row counts. The main table's byte counts cover its columns only, excluding those side tables. Startup-only movement bounds
have their own `preparation` owner and disappear after `free_preparation_only_data()`. No missing column is
replaced with zeros. Prepared display labels and immutable constellation definitions are shared from the catalog. Proper names and catalog
identifiers use the same label rules; `name_entry` references a pair of adjacent u32 boundaries.

Debug headers use descriptive labels, such as `initial_magnitude` and `brightest_possible_magnitude`, while Rust
fields retain their existing names. Column notes explain units, coordinate systems, index targets and reserved exception fields.
The source crosswalk is documented below `row_columns!` in `src/rows/mod.rs` and beside the contextual labels in
`src/state/tables/labels.rs`. Labels do not change data types, numerical precision or cache formats.

`display_color_index: u8` selects one of eight shared RGB/terminal palette entries, explained in the table notes.
Source spectral codes and B-V measurements are discarded after classification. The former renderer-side
`scene_cache.prepared.stars` table is removed; label eligibility comes from the catalog's existing name references.

`persistent.catalog.star_exceptions` is an empty skeleton for future motion-fallback and high-precision metadata.
Ordinary star rows no longer store flags or precision references. Datasets requiring these exceptions currently
fail with an explicit unsupported-feature error; the reserved precise-motion table must also remain empty.
Runtime fallback calculations for accepted stars outside the supported time interval are unchanged.

Used bytes describe live payload; reserved bytes include spare capacity. Both come from the original owners,
not slices of their data. Unknown sizes print `unknown`; notes identify partial counts such as nested allocations
and hash-table overhead. These entries are not a deduplicated process-memory total. The feature-gated inventory
below supplies detailed shared-owner accounting and per-step operations.

Preview ordering is prepared once per table. Only the first and last ten rows are formatted; nested collections
show at most four items, nesting is limited, and cell text is capped at 160 characters plus an omission marker.
These limits apply before hidden data is expanded. Every column remains present in the Markdown table, including wide tables. Pipes and markup in values are escaped.

Catalogs, including prepared-cache hits, use owned arrays. The disk cache is still used to avoid CSV parsing, but
its bytes are read, validated and decoded into arrays; no file mapping backs the tables. The temporary file-byte
buffer is released before loading returns. Loading can temporarily hold both bytes and decoded arrays. Names
and figures are shared without cloning their payloads. The source fingerprint changes once with this implementation, so existing prepared
caches rebuild. Used/reserved bytes are allocation measurements, not physical RAM residency; OS swapping is allowed.

## Build and run

Build support explicitly; passing the flag does not rebuild the binary:

```sh
cargo build --features memory-diagnostics --bin astroterm
```

In a terminal, inspect one reproducible frame with the small embedded catalog:

```sh
target/debug/astroterm -i Tokyo -d 2025-03-01T11:00:00 -s 0 -t 5 --debug-singleframe --debug-memory
```

For a continuous run, omit `--debug-singleframe`. Pan, zoom or resize, then press `q` to quit and read the report:

```sh
target/debug/astroterm -i Tokyo -d 2025-03-01T11:00:00 -s 1 -t 5 --debug-memory
```

For representative optimized calculations, build release support and replace `target/debug` with `target/release`:

```sh
cargo build --release --features memory-diagnostics --bin astroterm
```

Debug-build timing is not representative of release performance. Even a release build incurs instrumentation costs
when diagnostics are active. Compare runs using the same build, date, viewport, catalog and flags.

| Options | Behavior |
|---|---|
| Feature absent, `--debug-memory` | Error with rebuild guidance before terminal takeover |
| Feature present, no `--debug-memory` | No memory events or inventories collected |
| `--debug-memory --debug-singleframe` | Present one frame, restore terminal, report startup and frame |
| `--debug-memory` | Run normally; retain bounded evidence and report after quit |
| `--debug-singleframe` alone | Existing execution trace, without memory events |
| `--debug-frametimes` | Independent on-screen timing panel; memory reporting does not enable it |
| `--disable-cache --debug-memory` | Keep the existing cache bypass behavior; report actual operations |

The report appears after terminal cleanup, including handled rendering errors. An incomplete frame is labeled
separately and does not count as presented. The original operational error remains primary if reporting also fails.
Panic cleanup still restores the terminal where possible; a complete diagnostic report after a panic is not promised.

## Start with the inventory

Rows are grouped by **Catalog**, **Simulation**, **Observation**, **Projection**, and **Rendering**, with separate
configuration, diagnostic and external coverage. State owns the working buffers; algorithms receive narrow views.
A view grants access to existing data and usually allocates nothing itself.

A small illustrative row:

```text
state.cache.simulation.stars.stellar_scratch [Application/Heap] len=0 capacity=1024; used 0 B; reserved 152.0 KiB
```

This scratch vector is empty now, but keeps room for its next batch. Clearing it removed its elements; it did not
release its allocation. **Reserved includes used. Never add those two numbers.**

| Report term | Meaning |
|---|---|
| Root inline | The root value's embedded headers and fields; not all memory reachable from it |
| Used | Known live element payload, such as `len × size_of(element)` |
| Reserved | Known capacity payload, including the used portion |
| Application | Payload owned directly by the application tree |
| Shared | An `Arc` payload counted once in this snapshot; later references are aliases |
| Mapped logical bytes | File mapping length counted once; not heap usage or resident memory (RSS) |
| Borrowed / Alias | A reference to data already owned elsewhere; adds no owned bytes |
| Diagnostics | Known retained trace/report payload, separate from working data |
| External | Scoped terminal writer and other coverage outside the owned tree; often partial |
| Exact payload | Exact described element bytes, excluding allocator and OS overhead |
| Partial / lower bound | Some bytes known, other storage unmeasured—for example hash-table control bytes |
| Unknown | A size cannot be determined; it is not assumed to be zero |

Group subtotals partition the same owner totals; do not add both. Shared payload is attributed to the first inspected
path, and later paths show references. Nested element headers and their separately allocated payload are distinct.
Known contributions remain visible even if another contribution is unknown. Integer overflow is labeled separately.

Snapshots contain numbers and labels, not live references. “Captured before terminal cleanup” describes when the
measurement happened; it does not claim those bytes remain allocated when the report is printed. Rows named
`sample[n]` are inspection ordinals, not persistent identities. `[*]` groups inspected children after four examples;
it never extrapolates uninspected children.

Pixel text cells and their small label/dependency records remain in state between frames. Valid unchanged
production text borrows this completed grid; refresh resets cells so removed labels never remain visible.
Kitty retains completed RGB and one reusable RGBA work buffer. Readiness/version fields distinguish completed
results from invalid or partial work; retained capacity can increase steady memory. The sky-only raster cache remains
available for reuse and is borrowed directly during composition. Halfblocks require one short-lived image copy
at the external encoder's ownership boundary; graphics-image composition does not copy into an intermediate sky
buffer. Other ratatui/encoded results can still be retained for inspection; state ownership is not a promise of reuse. Designated scratch and some transport/metadata vectors keep capacity after clearing. This
work makes ownership inspectable; it does not claim lower memory usage, fewer allocations or faster frames.

## Read events beside processing times

Each timed step can list:

- **Borrow granted:** read-only or writable access, with element counts and index domain. This is permission, not
  evidence that every element was accessed.
- **Observed operation:** an instrumented copy, build, clear, reuse, comparison, cache store, move, buffer write or
  output call. Before/after shapes describe the recorded operation, not an allocator census.
- **Logical bytes:** the described payload. Moving a vector transfers its header/ownership; it does not copy its
  entire allocation. Reserved capacity does not prove whether reallocation occurred.

Index domains distinguish catalog indices, working selections, observed stars, visible stars and draw order. An index
from one list is not automatically meaningful in another. Operation counts describe repeated event observations, not
necessarily unique elements or the number of underlying syscalls. Batched stellar passes interleave; their events are
aggregated by category rather than recording every star. First/last shapes do not reconstruct intermediate states.

Timing is unsmoothed wall time. Parent times include children; do not sum both. Present measures writes and flushing,
not when the terminal finishes displaying a frame. Instrumented detail collection is reported separately where
possible; small inline counters remain in pass timing. Snapshot capture, report formatting and report output also
cost time. The frame duration through presentation excludes final inventory capture and later report output.

## Continuous retention and limits

Memory reporting retains **startup + the latest completed frame + the current partial frame**, not a history of every
frame. Each completed frame replaces the previous retained frame. The report also gives per-step elapsed sums and
invocation counts across completed frames; these are different from the normal smoothed timing panel. Partial failed
frames are not folded into completed-frame aggregates.

The initial limits are:

| Retained data | Limit |
|---|---:|
| Steps per startup/frame trace | 1,024 |
| Trace nesting depth | 32 |
| Retained event records per trace / per step | 8,192 / 128 |
| Detail records per trace | 4,096 |
| Detail text per trace / per record | 256 KiB / 4 KiB UTF-8 |
| Normal timing paths / aggregate paths | 512 / 1,024 |
| Inventories per startup/frame segment | 1 |
| Inventory rows / nesting depth | 4,096 / 16 |
| Inspected nested entries per container | 128 |
| Inventory field visits | 8,192 |

These timing-path and trace limits apply when memory run diagnostics are active. They do not cap the ordinary timing panel when the flag is absent. Limits cover retained records; a diagnostic callback can create a larger temporary string before it is truncated.

Reaching a limit reports omissions/truncation. Omitted details also count requests for a named step that was not
recorded, such as a disabled stage; this count alone does not mean a cap was exceeded. A missing row after truncation
is not proof that no work happened.
Flat vector payloads can be counted without visiting every element; nested allocations receive bounded inspection.
Known collector retained/scratch capacities and capture costs are reported. Diagnostic rows describe already retained
history at capture time; these figures overlap with per-capture collector figures and should not be summed blindly.

## Inspect the captured values in a debugger

`state.timings.memory_run()` exposes the typed run summary: completed frame count, startup trace, latest frame,
partial-frame time and per-step totals. `memory_inventories()` yields the captured inventory records. They own
numbers and labels rather than references into working buffers. This does not install or replace a CodeLLDB formatter.

## What still needs another tool

This report covers typed application buffers and explicit instrumentation sites. It does not intercept every read,
write, temporary allocation or hidden operation inside a library. It cannot establish peak heap, RSS, allocator
fragmentation, CPU cache misses or physical memory bandwidth. A whole-buffer equality comparison can do substantial
work while leaving every before/after size unchanged.

Use an allocation profiler such as Heaptrack to investigate allocation lifetimes and peaks, and a CPU/hardware
profiler for execution cost and memory traffic. Keep profiler results separate from this logical payload inventory.
The debugger remains useful for inspecting individual values. No global allocator hook or debugger extension is
installed by `--debug-memory`.

The processing trace separates **Solar-system simulation**, **Observer preparation**, **Star selection**,
**Stellar simulation**, and **Observation**, followed by projection and rendering. The first two finish body
sampling; star selection precedes motion calculations. The `Obs` cache counter retains its historical combined
meaning across observer preparation, selection, stellar samples and observation corrections.

Table paths follow these owners: `cache.simulation.solar_system`, `cache.simulation.stars`, `cache.observer`,
`cache.selection` and `cache.observation`. Views between stages are borrowed and do not duplicate these tables.
Older parent timing totals are not directly comparable after this split; compare complete frame times or matching
leaf calculations. Catalog cache fingerprints include source files, so the refactor causes a one-time prepared
catalog rebuild even though the stored catalog schema is unchanged.


### Regional stellar caches

Stellar freshness decisions count **regions**, not individual stars. `cache.simulation.stars.regions` owns one
slot per spatial region plus the always-requested constellation region. Every star belongs to exactly one group.
A refresh calculates all its stars, even those too faint for the current rendering threshold; diagnostics report
newly simulated rows separately from selected output rows. The effective TTL is displayed in simulated seconds.
Default is 864000 (10 days); this is a holding approximation, not an accuracy-certified interval. Local cache.toml
can shorten it. `--disable-cache` forces fresh results. Region epochs may differ; observation uses current time.

`Stellar region decisions` replaces per-star lookup/qualification. `Stellar batches` aggregates reusable output
preparation, trajectory reads, calculations and stores. Complete work is swapped into its region after a single
value comparison; equal results keep their generation. `StellarOutputWork` events describe reserve/append/clear,
while `StellarSamples` events describe regional reuse and commit. A reserve event can reuse existing capacity;
it does not necessarily mean a heap allocation. First population and capacity growth still allocate.

There is no `Motion output assembly` or combined motion table. Consumers borrow the original region samples.
`Selected fallback count` runs only when its selected rows or sample inputs change; the result is a scalar.
`cache.simulation.stars.region_output_work` shows retained scratch capacity even when its row count is zero.
`last_request` records completed selection/time provenance and never replaces regional calculation epochs.
On a cache hit there are no numerical batches. Old per-star/combined-output refresh counts and parent timing
totals are not directly comparable. Single-frame benchmarks measure cold population, not warm reuse.
The region table includes bounded nested sample previews and allocated capacity; the memory inventory aggregates
disjoint regional payloads into a bounded number of rows. Processing views are not duplicate tables.

### Regional observation selection

Current brightness and correction selection retain results only in `cache.observation.regions`. There are no
combined `cache.observation.eligible` or `cache.observation.corrections` tables. Observed stars are built directly
from regional correction records and borrowed stellar samples; their actual output buffer remains in state.
Counts retain the same meanings: evaluated working rows, skipped faint ordinary stars, and faint endpoints kept
for constellation geometry. Empty regions still have output descriptors.

The temporary correction-membership bridge has been replaced by `cache.observation.horizontal_sources`.
Its `regions` table stores region IDs, output spans, membership versions and apparent-result versions. A separate
`body_generation` records the solar-system input version, and `revision` is a dependency token. Metadata capacity
is reused; it contains no copied directions. `Horizontal request preparation` and `HorizontalRequest` events
report changes/reuse. The old brightness/correction assembly passes remain absent.

### Borrowed apparent directions

`cache.observation.apparent` and its combined star/body snapshot are removed. Stellar aberration remains in
original regional owners and solar-system aberration in `body_apparent`. Horizon rotation borrows these through
`ApparentDirections`, validates regional spans, and writes transformed output directly. The cached inputs remain
unchanged. Body aberration now has its own cache report; regional counters describe stellar aberration.

Under **Aberration**, there is no apparent cache decision, direction capture/store or direction restoration.
Under **Horizon rotation** and **Refraction**, refreshes now fill reusable work buffers and swap completed
results into their caches. Hits borrow existing results; there is no direction capture or restoration pass. Read-only RegionalApparent/BodyApparentDirections
borrow events describe rotation inputs; they do not imply copies or new allocations. This change does not claim
that every remaining direction copy has been removed.

### Borrowed final observation (Stage 7)

The production loop keeps `cache.sky.stars` empty. Its scalar metadata and small body fields are preparation
scratch; final star/body directions live in the horizontal/refraction caches and are exposed by a borrowed
ObservedSkyView. Correction records and magnitudes remain in their original regional caches. Views do not appear
as duplicate tables. `observe_cached_sky` / `materialize()` explicitly construct owned output when requested.

New original owners are `layout_sources`, `horizontal_work` and `refraction_work`. `published` records request
provenance; `use_refraction` identifies the selected final directions. Work arrays retain capacity after swaps.
`Observed region layout` compares small regional versions and does not rebuild star rows. On valid paused hits,
no Corrected-star buffer construction, direction calculation/capture/restoration, or owned materialization runs.
Other pipeline work, including raster keys, composition and presentation, is outside this unchanged-input shortcut.

HorizontalWork/RefractionWork events describe actual reserve/build/clear operations and result ownership moves.
Their logical shape combines two direction vectors and an inline Moon value; it is not one contiguous allocation.
Regional projected-cell addresses now contain a descriptor slot and logical observed row, both checked u32 values.


Production raster hits now inspect region/version metadata instead of rebuilding the star-sized key.
`scene_cache.pixel_inputs` is retained scratch, filled only for a redraw and cleared after success. Its reserved
bytes remain visible. `pixels.key.production.regions` lists the committed dependency records; candidate records
clear on a hit. Editable/headless callers retain exact star/geometry keys. Both modes share one cached sky image.
`Raster drawing inputs` reports actual per-star preparation on misses; `Raster dependencies` reports region
checks on every production call. Hits have no drawing-input build events. Separate text/RGB/encoding/display caches now cover later stages;
live timing/counter metadata can still change while simulation time is paused.


Completed-loop diagnostics distinguish new output from reuse of an already displayed Kitty image. The run report
shows `presented` and `reused display` totals for completed loops; partial/failed loops never enter these totals.
Single-frame mode still submits its first frame and reports one presentation. An unchanged Kitty frame has a
`Presentation decision`, but no encoding, upload, swap or `Present` step. Input polling and frame pacing continue.
An I/O failure invalidates the displayed state without overwriting its last successful identity or advancing the
image ID. Resize/explicit clearing and cache reconfiguration force presentation again. External terminal damage
outside those known events requires an explicit redraw; no terminal eviction detection is claimed.

Live frame-time metadata remains live. It can change text/RGB and force uploads while simulation time is paused.
Composition now follows text layout, so the timing panel samples the previous composition/conversion timings;
this avoids allocating a full image merely to discover its dimensions. New output caches use conservative
owner-local revisions; generic Cache value-generation semantics are unchanged. No panel throttling was added.


`compressor` is lazy retained zlib working storage, separate from `compressed` output bytes. Its allocator sizes
are opaque and reported as unknown coverage, not zero. `CompressionEngine` Build/Reuse events describe creation
or reset of the working engine on an actual encoding pass; they do not mean a previous compressed result was
reused. Each image remains an independent zlib stream. Encoding/display cache hits perform no engine operation.


The Kitty shared-memory prototype keeps the original RGB buffer and copies into at most one temporary POSIX
object. `shared_memory` reports selected transport; `shared_upload_bytes` exposes the pending object's logical
extent. This OS storage is reported separately as an external extent, not owned Rust heap, RSS or a client mapping.
Normal after-frame snapshots have no pending object. `SharedImage` Copy/Release events retain evidence of its
transient bytes. `Shared memory preparation` includes creation/copy/name encoding; `Shared memory consumption`
measures bounded waiting for the terminal's unlink. The small command-write time is not a display-latency claim.
On timeout or unavailable shared storage, the session falls back to normal streaming; failed writes remain errors.

Unavailable/unconfirmed Kitty shared memory also produces a yellow display notice, independent of `-m`.
Runtime fallback adds it on the next frame; its presence participates in text-cache validity.
