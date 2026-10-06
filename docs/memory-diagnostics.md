# Inspect working data and memory operations

`--debug-memory` reports which buffers the application keeps, their payload sizes, and the instrumented operations
performed by each pipeline step. It helps answer **“what data does this step work on?”** It is not a heap profiler or
a measurement of physical RAM traffic. Normal builds omit the collection and storage engine; the lazy hook API compiles to no-ops.

## Quick dump without the feature

Any build can write every table the state holds (catalog columns, caches, rendering buffers) with their shape,
used and reserved bytes, cache policy and metadata, column names and types, and the first and last ten rows:

```text
cache.sky.stars    shape=[1319]  used=61.8 KiB  reserved=61.8 KiB
  columns: source_index: usize | drawable: bool | magnitude: f64 | position: Vector3
  [0] 1 | true | 4.67 | Vector3 { x: 0.567, y: -0.798, z: 0.199 }
```

From code:

```rust
state.log_data(Some(Path::new("/tmp/astroterm-tables.log")), Some("after frame 3"))?;  // append a section
state.log_data(None, None)?;                                                            // print to stdout
```

Use a file path when the frame loop is running: the terminal owns stdout and the alternate screen is active.
The dump is the quick overview; the inventory below is the precise accounting of shared allocations, mappings,
coverage and per-step operations.

Catalogs, including prepared-cache hits, use owned arrays. The disk cache is still used to avoid CSV parsing, but
its bytes are read, validated and decoded into arrays; no file mapping backs the tables. The temporary file-byte
buffer is released before loading returns. Loading can temporarily hold both bytes and decoded arrays, and name
clones own separate storage. The source fingerprint changes once with this implementation, so existing prepared
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
state.cache.observation.scratch [Application/Heap] len=0 capacity=1024; used 0 B; reserved 152.0 KiB
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

Pixel images and ratatui results can be retained for inspection while still being rebuilt each frame. State ownership
is not a promise of reuse. Designated scratch and some transport/metadata vectors keep capacity after clearing. This
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
