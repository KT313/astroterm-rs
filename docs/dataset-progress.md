# Dataset progress and star counts

BSC, AT-HYG, local AT-HYG CSV/gzip and Gaia use the same startup progress display. Each keeps its own parser.
The display runs on stderr before terminal setup and ends before the sky renderer opens.

- A spinner shows activity while waiting for the download server. Downloads and existing Gaia checksum checks then show bytes processed.
- Parsing shows rows processed and retained sources where available; it does not scan the file first to count rows.
- Compact preparation and region assignment show processed rows; an opaque sort uses a spinner. Reordering
  counts copied field values across nine columns, not unique stars. Index construction reuses region counts
  and shows a spinner while preparing region coverage. Empty constellation inputs skip the endpoint scans.
- MAST shows “Planning dataset downloads” while locating the required blocks. Missing file metadata is
  fetched with up to four concurrent requests, independently of the block-download limit. Cached prefixes
  are checked locally without speculative network requests. A cold lookup may fetch up to three later file
  headers ahead of the selected prefix; no star blocks outside the selection are downloaded. Local raw-block
  checks use locations and sizes, without reading their payloads or claiming content verification.
- MAST uses one loading bar for all blocks. Counters, ETA and activity text appear on a separate, width-limited
  line so changing text cannot resize the bar. Activity shows cached-byte reads, checksum checks, decoding,
  conversion and assembly without resetting the completed-block count or its ETA. Source-ID checking
  then shows a sorting spinner followed by a scan count on the same bar.
- Prepared validation names its substeps. Its cumulative count covers two star passes (values/labels, then IDs
  and regional order), followed by scratch release and constellation checks; it is not a unique-star count.
- Cache writing counts uncompressed input bytes processed, then shows a finalization spinner through flush,
  sync and atomic installation. No stage claims successful completion before its work succeeds.
- Prepared-cache hits show reading and validation, without pretending to parse the source again.
- Multiple source files use one readiness bar and one current-task bar, not one bar per file.

`--no-progress` disables progress updates; warnings, errors and final loading notices remain. Redirecting
stderr, or using `TERM=dumb`, produces plain text: one line per stage change and at most one count line per second. An unusually long or explicitly flushed partial notice switches to plain output so redraws
cannot overwrite its text. Progress does not change download verification, cache
validity, numerical calculations or the simulation frame loop. No overall startup percentage is estimated.

`-m` also shows four star counts during simulation:

| Field | Meaning |
|---|---|
| Stars loaded | Usable stars in the installed catalog, recorded once at startup |
| In candidate regions | All stars in the conservatively selected regions, before brightness filtering |
| Passed filter | Drawable stars after brightness checks, including the current-magnitude check |
| Passed projection | Drawable stars projected inside the visible view |

Counts use commas: `10,221,029 stars`. They exclude the Sun, Moon and planets. Candidate regions include
the always-requested constellation group once; endpoint-only stars that fail brightness checks do not enter
the final two counts. Retained offscreen cache regions do not enter the current counts.

Projection count is not a pixel count: overlapping stars count separately. Later pixel-edge clipping,
metadata coverage or another object painting over a star does not change it. These are the results used
for the displayed frame, including valid reused samples. Paused frames read stored counts rather than
scanning the stars again.

`--debug-singleframe` also records startup substeps for cached-block reads, row/ID assembly, capacity changes,
ID sorting/checking, region assignment, sorting, permutation, indexing and prepared-cache validation/writing.
Parent times include their children; do not add them together. Timings include progress-reporting overhead.

A local raw-block repair keeps the loading bar active without resetting its completed-block count or elapsed
time. Its activity text identifies the repair; failure is reported if the original pinned source is unavailable.

## Startup sorting

Catalog preparation uses one worker pool for sorts of at least 1,000,000 rows. It uses up to four threads,
capped by available CPU parallelism, with a minimum of one. With only one available CPU, sorting runs on the
calling thread. There is no command-line or environment override for the thread count.

Gaia source-ID checking and region/brightness sorting share that pool. Record construction and column
reordering remain serial; downloads and frame rendering keep their existing behavior. Smaller sorts remain
serial, and prepared-cache hits skip sorting entirely. `--disable-cache` only disables runtime caches and
does not force catalog preparation.

`--debug-singleframe` reports actual worker count, sort timings and record-buffer size.

Metadata planning consumes files in catalog order even when later requests finish first. Workers do not draw
progress bars or add overlapping durations to the trace: `MAST metadata planning` measures caller wall time,
with `MAST metadata wait` recording time waiting for the next required file. Required failures stop planning;
unused lookahead failures do not reject a valid shorter selection. All workers are joined before proceeding.
Cancellation stops new work and retry waits; active HTTP reads keep their existing timeouts.
