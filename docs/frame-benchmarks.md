# Bounded frame timing captures

Capture a fixed number of frames with reproducible simulated dates:

```sh
mkdir -p tmp
target/release/astroterm -i Tokyo -d 2025-03-01T11:00:00 -m \
  --graphics-protocol kitty --fps 10000 \
  --debug-frames 101 --debug-frame-step-seconds 0 \
  --debug-frame-report tmp/frames-001.jsonl
```

The report file must be new; existing files are not overwritten. Its parent directory must exist.
The program exits after 101 completed frames, including frames which reuse an already displayed image.
There is no need to wait for 101 uploads. Terminal settings are restored before the successful end record.

`--debug-frame-step-seconds` defaults to zero (paused). Positive or negative values advance or reverse by
that many simulated seconds per frame. Decimal input is exact to milliseconds (at most three fractional digits);
exponent notation and finer fractions are rejected. Frame 1 uses the exact requested date; later dates are calculated
from that starting date and the frame index, independently of processing speed or report I/O delays.
Metadata shows `paused` or a signed number of seconds per frame. Ordinary interactive clock behavior is unchanged.

Do not combine this mode with `--speed`, `--debug-singleframe`, or `--debug-log-data`.
Quit, a resize, or recognized view/time controls abort capture with a nonzero exit status. An incomplete
run is not a successful benchmark. `--debug-singleframe` retains its existing human-readable report.

The visible `--debug-frametimes` panel is optional and independent of capture. Its changing numbers become
part of the rendered image and can prevent display reuse in a paused simulation. Leave it off to measure
cache reuse without that extra rendering work. Metadata (`-m`) can still be shown.

## Reading the report

Each flushed JSONL record is available while the program runs. Transport schema version 2 contains:

- `run`: count, integer `step_ms`, integer `start_time_ms`, the year-zero epoch/unit/time-scale convention, actual renderer/geometry/transport and effective configuration. The
  Kitty `transport` is `shared_memory` or `streaming`; each frame record carries the transport in force for that
  frame, since a terminal that does not consume a shared object switches the run to streaming.
- `startup`: raw startup steps, including catalog and frame preparation.
- `frame`: contiguous one-based index, integer `simulation_time_ms`, real elapsed seconds, presentation outcome, actual display state,
  and raw timing steps. `reused_display` is a successful completed frame even without a `Present` step.
- `end`: `complete`, `aborted`, or `failed`, with requested/completed/presented/reused counts. Startup or
  report-write failure may leave an incomplete file; a missing end record must never be treated as success.

Step records contain a name, parent index within the same segment, raw seconds, and directly recorded
diagnostic seconds. Startup and frame checkpoints use dotted paths, such as `Dataset loading.Catalog preparation`
and `Solar system.Earth state`. Repeated paths aggregate into one row per capture segment. Any number of
path levels is supported; the serialized rows retain separate names and parent indices. These are unsmoothed
measurements, not the timing panel's moving averages. Parent durations include children: do not add them together.

Each checkpoint closes the preceding interval and starts the next. All intervening work belongs to that interval,
including loop bookkeeping and function returns. A top-level checkpoint also accounts for work outside its named
children. `END` closes the final interval without starting another, before report I/O and FPS sleep. Frame totals
are published together at `END`, so the visible panel shows the previous completed frame's smoothed measurements
and has no frame timing rows on the very first frame. Without detailed diagnostics, only top-level transitions
read the clock; repeated child checkpoints within the same stage do not.

Frame elapsed time ends after rendering/presentation and before final reporting. Startup, report serialization,
flush, trace cleanup, and FPS sleep are excluded. Existing in-frame instrumentation is included. Report I/O
can still affect overall process time and CPU conditions; capture is a diagnostic workload. Only one frame's
trace is retained in Rust. No cache buffers are cleared between frames. Presentation measures writes/flush,
not physical display completion.

## Local benchmark helper

The owner's gitignored `dev/bench.py` consumes these reports through the existing real-terminal `script` relay:

```sh
python3 dev/bench.py 5 --frames 101 --step-seconds 1 --name base
python3 dev/bench.py 5 --frames 101 --step-seconds 1 --compare-to base
python3 dev/bench.py compare base updated
```

The positional count still means independent processes. Defaults are three runs, one frame per run, paused
simulation and no visual timing panel; `--show-frametimes` includes the panel explicitly. No warmup frames
are discarded. For five runs of 101 frames, first-frame results have five samples and subsequent results
have 500. Startup is summarized separately. One-frame results have no subsequent-frame average.

Per-frame stage tables show amortized cost, conditional cost when executed, and frames containing the stage.
Known cache-skipped work contributes zero to the amortized cost; absent/unknown or incomplete measurements
are not fabricated as zero. The helper preserves all raw frame samples in saved-result schema v2 and still
loads v1 first-frame results. Legacy comparisons disclose missing metadata and different terminal recording
conditions. New captures do not log the graphics transcript to disk. Named-result overwrite protection stays
in effect; use `--overwrite` explicitly. The helper is local development tooling and is not shipped in Git.

### Frame sub-step timings

The seven main stages have child rows in the live `--debug-frametimes` panel, single-frame traces and bounded
frame captures. Region refresh separates brightness-prefix lookup, simulation caches, image cache decisions,
group drawing and region merging. Whole-tile forward projection separates the star layer clear, tile geometry,
tile-corner projection, Adaptive mesh preparation (or Hermite/Coons when selected), filter preparation, tile rasterization and region composition. Output separates composition/comparison, RGB
conversion, shared-memory preparation/consumption, or encoding/upload for streaming protocols.

Repeated region and texel passes publish one accumulated child row per pass per frame, in declared pass order,
not one trace row per iteration. Unexecuted passes stay absent from raw captures; the panel's existing averages
decay when work is skipped. Parent durations include children and timing overhead: do not add parents to their
children. Fine-grained timers have a cost, especially for texel loops. They run only for the timing panel, traces
or benchmark capture; ordinary rendering retains the seven top-level measurements without reading sub-step clocks.

The straight, Coons and Hermite backends can be compared without external catalog downloads with:
`cargo test --release --lib compare_coons_mapping -- --ignored --nocapture`.
This uses embedded BSC5 plus a synthetic dense-tile variant; it does not measure terminal upload or AT-HYG.

The adaptive prototype has a normal/extreme-view timing fixture:
`cargo test --release --lib time_adaptive_mapping -- --ignored --nocapture`.
It uses the screenshot camera/date at 180 and 351.6 degrees, no refraction, embedded BSC5 and synthetic dense tiles.
Detailed frame traces also report the number of real sky projections, maximum subdivision and cells hitting
limits. These counts describe mesh construction; they do not certify all screen pixels against the real formula.

The adaptive timing fixture prints three untraced projection times per case (averaged after warmup), followed
by child-stage times from a separate traced frame. The child times are not components of that untraced average.

Simulation timestamps use signed milliseconds since `0000-01-01T00:00:00`, with uniform Gregorian days
and astronomical year numbering. New saved benchmark results use version 3. Historical saved results
remain readable for comparisons, but their floating-point dates are not silently rounded into the new
schedule. Performance durations remain real floating-point seconds/milliseconds.
