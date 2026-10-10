# Gaia DR3 subsets

```sh
astroterm --dataset gaia-gdr3 --dataset-files 1 -i Tokyo -t 10 -c
```

Choose exactly one of `--dataset-files` (ESA bulk files) or `--dataset-stars` (selective MAST Parquet).
Both accept zero for no catalog stars. `--dataset-files` accepts **0–3386**. Each main-source file contains roughly
half a million sources and is roughly 230 MB compressed. Files are selected in fixed numeric sky-region
order from the bundled ESA checksum manifest. They are **not a random or brightness-ranked sample**;
with a few files, most of the sky has no Gaia stars. The first file covers an area near RA 45°, Dec 0°.

Increasing the count from 5 to 8 downloads only the three missing files. Decreasing it loads only the
requested prefix and leaves other downloads intact. Zero means no catalog stars, while the Sun, Moon,
planets and controls still work. The flag is rejected for other datasets. A Gaia run loads **only Gaia**;
it does not supplement stars from AT-HYG or BSC. Names are `Gaia DR3 <source_id>`. Constellation figures
are unavailable because these files do not provide our HR endpoint identifiers.

## Selective MAST downloads

```sh
astroterm --dataset gaia-gdr3 --dataset-stars 1000000 -i Tokyo -t 10 -c
```

`--dataset-stars` accepts **0–4,294,967,295** raw input rows from MAST's full DR3 main-source table.
It downloads only 13 columns: source ID, designation, position/epoch/motion/distance inputs and G/BP/RP
magnitudes. Labels use the supplied `designation` text. HTTP byte-range reads fetch compressed Parquet
column chunks directly; there is no TAP query queue. Building requires a C toolchain for Zstandard.

Selection follows pinned HATS sky partitions, then Parquet row-group and physical row order. It is **not
sorted by source ID**, random, or brightness-ranked, and differs from an ESA file prefix. Each final row
group is fetched in full (200,000 rows in the first groups); unselected rows stay on disk for later use.
A request for one star can therefore download a whole group's selected columns. Loaded stars may be fewer
than requested because required measurements are missing or the provider contains duplicate copies.

The mirror has a known triplicated region. Astroterm removes adjacent copies only when all 13 consumed
values/nulls match (numeric comparison uses bit patterns), including across group boundaries. Conflicting
copies or unexpected nonadjacent duplicates cause an error. Copies still count toward the raw requested
number and keep their physical ordinals; the loading report separates duplicates from missing measurements.

Increasing the count reuses existing raw and converted blocks. Only new blocks are downloaded and converted.
A conversion-policy change rebuilds converted blocks from local Parquet, without redownloading it. Final
catalog loading, copying, validation, sorting and indexing still run for a changed selection. A repeat of
the same selection can use its whole-catalog prepared cache. Reducing the count keeps extra files intact.
The program never automatically evicts these caches: **each distinct requested count can create another
full prepared catalog**, and replaced content-addressed objects can remain on disk. Remove unwanted caches
manually while astroterm is closed if disk space is needed.

Unlike the optional whole-catalog cache in bulk-file mode, nonzero `--dataset-stars` requires a usable
cache directory: reusable converted blocks are part of this mode's contract. Missing or unwritable converted
storage fails explicitly. A failure to save the optional final whole-selection cache remains a notice.
Zero does not need either directory.

MAST defaults on Linux:

- Pinned metadata, raw blocks and atomic references: `~/.local/share/astroterm/gaia-gdr3/mast-parquet-v1/`
- Converted blocks: `~/.cache/astroterm/gaia-gdr3/mast-converted-v1/`
- Combined prepared catalogs: `~/.cache/astroterm/gaia-gdr3-mast-<fingerprint>.catalog`

Files are stored under content hashes; small references are published only after complete verified writes.
Interrupted work leaves completed blocks reusable and restarts the incomplete block. A damaged discovery
or footer object is refetched and restored only if it matches the original pinned identity; changed remote
metadata is never silently adopted. Cache errors include the local path. Checksums detect local
corruption; they are not independent publisher checksums. Metadata is pinned locally and partition ETags
are pinned when first accessed. A changed pinned object fails explicitly rather than mixing revisions.
Unvisited files are pinned lazily, so this is not a provider-guaranteed atomic historical snapshot. Complete
local selections work offline. There is no automatic switch to ESA or another mirror on a network failure.

Whole-group conversion also records per-row errors in an unselected suffix. An unsupported star is reported
when the requested prefix actually reaches it; it is not silently removed. Storage or malformed-format
errors still fail immediately. Zero performs no network access. If N exceeds the full catalog, the loader
prints the available raw count and loads what exists. `--disable-cache` still affects runtime caches only.

## Brightness, colors and motion

**Gaia G magnitude is stored in the existing V-magnitude field, without G-to-V conversion.** A startup
message explains this on every Gaia run, including cache hits. Threshold filtering and rendering therefore
use G. Existing table columns and rendering controls are unchanged. Color uses observed BP/RP measurements
mapped to the existing coarse color palette, using the DR3 Johnson-Cousins relationship; this is a display
approximation, not a new measurement of B-V. Missing color uses the default palette color.

The importer converts degrees and mas/year to the program's units and rebases the J2016 straight-line
trajectory to J2000. It approximates Gaia's TCB Julian years by TT Julian years, and uses the existing
straight-line motion model, not Gaia's full scientific processing model. Mean G brightness is adjusted for
reference-distance changes when a usable distance exists. Stellar variability is not simulated.

- Positive parallax with signal/error at least 10 gives the approximate distance `1000/parallax_mas` pc,
  within the existing distance range. Negative, absent or insufficiently significant parallaxes use
  angular-only motion and constant intrinsic brightness. No Bayesian distance or extinction tables are fetched.
- Missing radial velocity is assumed zero when a usable distance and proper motion exist.
- If either proper-motion component is missing, the direction stays fixed. Both five- and six-parameter
  astrometric solutions can be imported; pseudocolour is not used as a display color.
- Missing position or G brightness removes a row, with counts printed during import. Malformed consumed
  values fail with file/row context. Sources are not filtered by star/galaxy classification or a quality cut.
- Existing limits on near-collision handling and high-precision trajectories still apply. Unsupported
  sources cause an explicit error rather than being silently removed.

## ESA bulk download and prepared-cache storage

All datasets share the startup progress display, which finishes before the sky renderer opens.
Use `--no-progress` to hide updates while keeping warnings and loading summaries. Redirected stderr gets
plain text. See [dataset progress and star counts](dataset-progress.md).

On Linux, default locations are:

- Raw inputs: `~/.local/share/astroterm/gaia-gdr3/gaia_source/`
- Prepared catalogs: `~/.cache/astroterm/gaia-gdr3-<fingerprint>.catalog`

`XDG_DATA_HOME` and `XDG_CACHE_HOME` override the platform directories. Existing selected files are checked
against ESA's MD5 checksums at startup; this reads the files but does not download them again. MD5 detects
transfer corruption, not cryptographic authenticity. Failed downloads leave completed files reusable;
the incomplete file restarts next time. Only verified files are atomically installed.

Parsing streams one file at a time into compact owned catalog columns. A prepared catalog can skip parsing,
conversion and preparation on subsequent runs. Its identity covers the selected prefix, verified inputs
and conversion rules. Increasing the count rebuilds that prepared catalog; it does not incrementally merge
old prepared results. `--disable-cache` affects runtime calculation reuse, not source downloads.

All runtime catalog data is loaded into RAM. Selecting all files can require hundreds of gigabytes including
preparation and runtime buffers; the allowed file count is not a promise that any machine can load it.
Label byte offsets use u64 to avoid a 4 GiB text-buffer limit; internal star/name IDs remain checked u32.
Old prepared catalogs rebuild once because the label-boundary format changed.

## Sources and acknowledgement

- [MAST/STScI Gaia DR3 HATS catalog](https://registry.opendata.aws/mast-gaia-dr3/) — prepared by the LSST Interdisciplinary Network for Collaboration and Computing; see [LSDB acknowledgements](https://docs.lsdb.io/en/latest/citation.html).
- [Gaia DR3 main-source files](https://cdn.gea.esac.esa.int/?prefix=Gaia/gdr3/gaia_source/)
- [Field definitions](https://gea.esac.esa.int/archive/documentation/GDR3/Gaia_archive/chap_datamodel/sec_dm_main_source_catalogue/ssec_dm_gaia_source.html)
- [Photometric relationships, Tables 5.9–5.10](https://gea.esac.esa.int/archive/documentation/GDR3/Data_processing/chap_cu5pho/cu5pho_sec_photSystem/cu5pho_ssec_photRelations.html)
- [Gaia DR3 summary paper](https://doi.org/10.1051/0004-6361/202243940)
- [Gaia mission paper](https://doi.org/10.1051/0004-6361/201629272)

This work uses data from the European Space Agency (ESA) mission Gaia, processed by the Gaia Data Processing
and Analysis Consortium (DPAC). Funding for DPAC has been provided by national institutions, in particular
those participating in the Gaia Multilateral Agreement. See [Gaia data credits](https://www.cosmos.esa.int/web/gaia/credits).
The repository includes the small publisher file/checksum manifest with provenance, not the Gaia star files.
