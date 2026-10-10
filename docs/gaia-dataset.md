# Gaia DR3 subsets

```sh
astroterm --dataset gaia-gdr3 --dataset-files 1 -i Tokyo -t 10 -c
```

`--dataset-files` is required for Gaia and accepts **0–3386**. Each main-source file contains roughly
half a million sources and is roughly 230 MB compressed. Files are selected in fixed numeric sky-region
order from the bundled ESA checksum manifest. They are **not a random or brightness-ranked sample**;
with a few files, most of the sky has no Gaia stars. The first file covers an area near RA 45°, Dec 0°.

Increasing the count from 5 to 8 downloads only the three missing files. Decreasing it loads only the
requested prefix and leaves other downloads intact. Zero means no catalog stars, while the Sun, Moon,
planets and controls still work. The flag is rejected for other datasets. A Gaia run loads **only Gaia**;
it does not supplement stars from AT-HYG or BSC. Names are `Gaia DR3 <source_id>`. Constellation figures
are unavailable because these files do not provide our HR endpoint identifiers.

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

## Download and prepared-cache storage

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

- [Gaia DR3 main-source files](https://cdn.gea.esac.esa.int/?prefix=Gaia/gdr3/gaia_source/)
- [Field definitions](https://gea.esac.esa.int/archive/documentation/GDR3/Gaia_archive/chap_datamodel/sec_dm_main_source_catalogue/ssec_dm_gaia_source.html)
- [Photometric relationships, Tables 5.9–5.10](https://gea.esac.esa.int/archive/documentation/GDR3/Data_processing/chap_cu5pho/cu5pho_sec_photSystem/cu5pho_ssec_photRelations.html)
- [Gaia DR3 summary paper](https://doi.org/10.1051/0004-6361/202243940)
- [Gaia mission paper](https://doi.org/10.1051/0004-6361/201629272)

This work uses data from the European Space Agency (ESA) mission Gaia, processed by the Gaia Data Processing
and Analysis Consortium (DPAC). Funding for DPAC has been provided by national institutions, in particular
those participating in the Gaia Multilateral Agreement. See [Gaia data credits](https://www.cosmos.esa.int/web/gaia/credits).
The repository includes the small publisher file/checksum manifest with provenance, not the Gaia star files.
