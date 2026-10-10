# Dataset progress and star counts

BSC, AT-HYG, local AT-HYG CSV/gzip and Gaia use the same startup progress display. Each keeps its own parser.
The display runs on stderr before terminal setup and ends before the sky renderer opens.

- Downloads and existing Gaia checksum checks show bytes processed.
- Parsing shows rows processed and retained sources where available; it does not scan the file first to count rows.
- Compact preparation shows a known entry total. Sorting and indexing use spinners.
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
